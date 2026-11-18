use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Local};
use tokio::sync::mpsc;

use crate::actions::{Host, Review};
use crate::memory::Memory;
use crate::routines::{self, Routine};
use crate::skills::Skills;
use crate::slug::slug;
use crate::subagents::{Report, Subagents};
use crate::web::Web;
use crate::{
    ActionRequest, ActionSpec, Approval, Channel, Message, Provider, ProviderError, Request, Store,
    StoreError,
};

/// Who the Agent is, told to the model on every turn.
const INSTRUCTIONS: &str = "\
You are a personal AI agent running on a machine that belongs to your Owner, \
the only person you talk to and work for. You can act on that machine with the \
actions you are given; use them rather than guessing or asking your Owner to \
do things you can do yourself.

Some actions need your Owner's approval. Never ask for that permission in \
words: request the action directly, and your Owner is shown exactly what it \
would do, with buttons to allow or deny it. When an action is denied, do not \
try to reach the same end another way.

With `schedule_routine` you set work for later: a reminder or a follow-up \
once, or something recurring. Use it whenever your Owner asks to be reminded \
or wants something done regularly, rather than promising to remember.

A notice is something you are told that your Owner did not write, such as \
the report of a sub-agent or of a routine's run: your Owner has not seen it, so tell \
them what matters in it.

Answer in the language your Owner writes in. Be direct and brief.";

const MEMORY_INSTRUCTIONS: &str = "\
## Your memory

Your memory outlives every conversation. Use `remember` for what will still \
matter later about your Owner (who they are, what they prefer, what they are \
working on, how they want things done), and not for passing details. Keep one \
topic per entry, under a name that says what it is about: their name, their \
studies and their preferences are three entries, not one.

To correct an entry, or to drop only part of it, `remember` it again under the \
same name with the text as it should now read. Use `forget` only when the \
whole entry must go. Your Owner can read all of it, so tell them plainly what \
you remember when asked.

What you remember now, as name: text.";

const SKILLS_INSTRUCTIONS: &str = "\
## Your skills

A skill is a way of doing something that you keep for later. Before doing \
anything one of your skills covers, load it with `use_skill` and follow it. \
When you have worked out how to do something your Owner will want again, \
offer to keep it with `save_skill`; when a skill turned out wrong or \
incomplete, save it again improved.

Your skills now, as name: what it is for.";

const SUMMARY_HEADING: &str = "\
## Earlier in this conversation

The start of your conversation with your Owner is no longer shown to you in \
full. This is what it held; use `search_conversation` when you need the exact \
words of something said before.";

/// Stands in for the Owner's opening words when they are only summarised.
const SUMMARISED_NOTE: &str =
    "[The earlier part of our conversation is summarised in your instructions.]";

const SUMMARY_INSTRUCTIONS: &str = "\
Write a summary of a conversation between a personal AI agent and its Owner, \
for the agent to read in place of the conversation itself. Keep what the agent \
needs to carry on: what the Owner asked for and why, decisions made, what was \
done and what came of it, what is still pending, and facts, names, numbers and \
preferences the Owner gave. Leave out pleasantries and anything already \
settled and of no further use. Write plain prose in the Owner's language, a \
few short paragraphs at most. Reply with the summary only.";

/// Channels show a working signal for a few seconds only; it is renewed at
/// this pace while the Agent works.
const WORKING_SIGNAL_EVERY: Duration = Duration::from_secs(4);

/// A Routine started this long after it was due is announced as late.
const LATE_AFTER: Duration = Duration::from_secs(300);

/// How many times a busy Provider is asked before the Owner is told.
const TRIES_WHILE_BUSY: usize = 5;

/// The longest a busy Provider is waited for between two tries.
const LONGEST_WAIT: Duration = Duration::from_secs(30);

/// How many times in a row the model may act before the Agent stops and
/// hands back to its Owner.
const STEPS_BEFORE_STOPPING: usize = 50;

/// The single persistent AI worker of an installation.
pub struct Agent<P> {
    provider: Arc<P>,
    store: Store,
    host: Arc<Host>,
    /// The most the model is sent on a turn, in tokens (estimated).
    context_budget: usize,
    subagents: Subagents,
    /// Where the reports of Sub-agents and of Routine runs arrive.
    reports: mpsc::UnboundedReceiver<Report>,
    /// Tells the date and time where the Owner is.
    clock: Arc<dyn Fn() -> DateTime<FixedOffset> + Send + Sync>,
    /// How often the Agent looks at the time for Routines that are due.
    time_check: Duration,
    /// True when a notice was added that the Agent has yet to respond to.
    owed: bool,
}

impl<P: Provider + 'static> Agent<P> {
    pub fn new(provider: P, store: Store) -> Self {
        let host = Host {
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            command_timeout: Duration::from_secs(120),
            memory: None,
            skills: None,
            web: Web::new(),
        };
        let (subagents, reports) = Subagents::new();
        Self {
            provider: Arc::new(provider),
            store,
            host: Arc::new(host),
            context_budget: 60_000,
            subagents,
            reports,
            clock: Arc::new(|| Local::now().fixed_offset()),
            time_check: Duration::from_secs(20),
            owed: false,
        }
    }

    /// The Host, while the Agent is still being set up.
    fn host_mut(&mut self) -> &mut Host {
        Arc::get_mut(&mut self.host).expect("the Agent is set up before it starts working")
    }

    /// Where the Agent reads the date and time from.
    pub fn telling_time_with(
        mut self,
        clock: impl Fn() -> DateTime<FixedOffset> + Send + Sync + 'static,
    ) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// How often the Agent looks at the time for Routines that are due.
    pub fn checking_time_every(mut self, pace: Duration) -> Self {
        self.time_check = pace;
        self
    }

    /// How many turns a Sub-agent gets before it must report.
    pub fn giving_subagents_steps(mut self, steps: usize) -> Self {
        self.subagents.steps = steps;
        self
    }

    /// The folder where the Agent's commands run and its relative paths start.
    pub fn working_in(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.host_mut().workspace = workspace.into();
        self
    }

    /// The folder where the Agent keeps its Memory, one file per entry.
    pub fn remembering_in(mut self, folder: impl Into<PathBuf>) -> Self {
        self.host_mut().memory = Some(Memory::kept_in(folder.into()));
        self
    }

    /// The folder where the Agent keeps its Skills, one folder per Skill.
    pub fn learning_in(mut self, folder: impl Into<PathBuf>) -> Self {
        self.host_mut().skills = Some(Skills::kept_in(folder.into()));
        self
    }

    /// The most the model is sent on a turn. Orbite counts four characters
    /// to a token, which is close enough for every Provider. Beyond it, the
    /// oldest part of the Conversation is replaced by a summary.
    pub fn keeping_context_under(mut self, tokens: usize) -> Self {
        self.context_budget = tokens;
        self
    }

    /// The address of the search engine's results page.
    pub fn searching_at(mut self, address: impl Into<String>) -> Self {
        self.host_mut().web.search_at = address.into();
        self
    }

    /// A command still running after `limit` is stopped.
    pub fn stopping_commands_after(mut self, limit: Duration) -> Self {
        self.host_mut().command_timeout = limit;
        self
    }

    /// Holds the Conversation on `channel` until the Channel closes.
    pub async fn converse(&mut self, channel: &mut impl Channel) {
        // Whatever was left unfinished when Orbite last stopped comes first.
        if self.unfinished().unwrap_or(false) && !self.respond_on(channel).await {
            return;
        }
        let mut time_check = tokio::time::interval(self.time_check);
        time_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let text = tokio::select! {
                written = channel.receive() => match written {
                    Some(text) => text,
                    None => return,
                },
                Some(report) = self.reports.recv() => {
                    if self.note(report).is_ok() && !self.respond_on(channel).await {
                        return;
                    }
                    continue;
                }
                _ = time_check.tick() => {
                    let _ = self.run_due_routines();
                    if std::mem::take(&mut self.owed) && !self.respond_on(channel).await {
                        return;
                    }
                    continue;
                }
            };
            if text.trim() == "/reset" {
                let said = match self.store.reset() {
                    Ok(()) => "Started afresh. What I remember about you is unchanged.".to_owned(),
                    Err(error) => explain_storage(&error),
                };
                if channel.send(&said).await.is_err() {
                    return;
                }
                continue;
            }
            if let Err(error) = self.store.append(&Message::from_owner(text)) {
                if channel.send(&explain_storage(&error)).await.is_err() {
                    return;
                }
                continue;
            }
            if !self.respond_on(channel).await {
                return;
            }
        }
    }

    /// Who the Agent is, what it remembers and what it knows how to do, as
    /// told to the model.
    fn instructions(&self) -> String {
        let mut instructions = INSTRUCTIONS.to_owned();
        if let Some(memory) = &self.host.memory {
            instructions.push_str(&format!("\n\n{MEMORY_INSTRUCTIONS}\n"));
            let entries = memory.entries();
            if entries.is_empty() {
                instructions.push_str("\n(nothing yet)");
            }
            for (name, text) in entries {
                instructions.push_str(&format!("\n- {name}: {text}"));
            }
        }
        if let Ok(Some(summary)) = self.store.summary() {
            instructions.push_str(&format!("\n\n{SUMMARY_HEADING}\n\n{summary}"));
        }
        if let Some(skills) = &self.host.skills {
            instructions.push_str(&format!("\n\n{SKILLS_INSTRUCTIONS}\n"));
            let skills = skills.all();
            if skills.is_empty() {
                instructions.push_str("\n(none yet)");
            }
            for skill in skills {
                instructions.push_str(&format!("\n- {}: {}", skill.name, skill.description));
            }
        }
        instructions.push_str(&format!(
            "\n\n## Now\n\nWhere your Owner is, it is {}.",
            (self.clock)().format("%A %Y-%m-%d %H:%M (UTC%:z)")
        ));
        instructions
    }

    /// True when the Conversation ends on something still owed a response.
    /// Actions that were cut short are first given an Outcome saying so.
    fn unfinished(&self) -> Result<bool, StoreError> {
        let conversation = self.store.conversation()?;
        let answered = |id: &str| {
            conversation
                .iter()
                .any(|message| matches!(message, Message::Outcome { action, .. } if action == id))
        };
        let mut cut_short = false;
        for message in &conversation {
            let Message::Agent { actions, .. } = message else {
                continue;
            };
            for action in actions.iter().filter(|action| !answered(&action.id)) {
                cut_short = true;
                self.store.append(&Message::Outcome {
                    action: action.id.clone(),
                    text: "Orbite stopped before this action finished. It was interrupted: \
                           whether it had any effect is unknown."
                        .into(),
                })?;
            }
        }
        Ok(cut_short
            || matches!(
                conversation.last(),
                Some(Message::Owner(_) | Message::Outcome { .. } | Message::Notice(_))
            ))
    }

    /// Responds to the latest of the Conversation; false once the Channel is closed.
    async fn respond_on(&mut self, channel: &mut impl Channel) -> bool {
        match self.respond(channel).await {
            Ok(open) => open,
            Err(error) => channel.send(&explain_storage(&error)).await.is_ok(),
        }
    }

    /// Thinks and acts, turn after turn, until the model has nothing left to do.
    async fn respond(&mut self, channel: &mut impl Channel) -> Result<bool, StoreError> {
        let mut actions = self.host.specs();
        actions.push(search_spec());
        actions.extend(Subagents::specs());
        actions.extend(routines::specs());
        for _ in 0..STEPS_BEFORE_STOPPING {
            self.fit(channel, &actions).await?;
            let mut conversation = self.store.conversation()?;
            // Every protocol expects the Owner to speak first.
            let opened = !matches!(
                conversation.first(),
                Some(Message::Owner(_) | Message::Notice(_)) | None
            );
            if opened {
                conversation.insert(0, Message::from_owner(SUMMARISED_NOTE));
            }
            let instructions = self.instructions();
            let request = Request {
                instructions: &instructions,
                conversation: &conversation,
                actions: &actions,
            };
            let mut said = Vec::new();
            let thinking = patiently(&*self.provider, request);
            let reply = match attending(channel, thinking, &mut said, true).await {
                Attended::Stopped => return self.stopped(channel).await,
                Attended::Interrupted => {
                    // The Owner added something: think again with it.
                    for text in said {
                        self.store.append(&Message::from_owner(text))?;
                    }
                    continue;
                }
                Attended::Done(Ok(reply)) => reply,
                Attended::Done(Err(error)) => {
                    // The Owner will say it again: a message the Provider
                    // failed on must not stay and be answered twice.
                    if matches!(conversation.last(), Some(Message::Owner(_))) {
                        self.store.forget_latest()?;
                    }
                    return Ok(channel.send(&explain(&error)).await.is_ok());
                }
            };
            self.store.append(&Message::Agent {
                text: reply.text.clone(),
                actions: reply.actions.clone(),
            })?;
            if !reply.text.is_empty() && channel.send(&reply.text).await.is_err() {
                return Ok(false);
            }
            if reply.actions.is_empty() {
                return Ok(true);
            }
            // Once the Owner stops or redirects the work, the Actions not yet
            // started are left out, each with an Outcome saying why.
            let mut left_out: Option<&str> = None;
            let mut stopped = false;
            for action in &reply.actions {
                let outcome = |text: String| Message::Outcome { action: action.id.clone(), text };
                if let Some(why) = left_out {
                    self.store.append(&outcome(why.into()))?;
                    continue;
                }
                if action.name == "search_conversation" {
                    self.store.append(&outcome(self.search(action)))?;
                    continue;
                }
                let named = |field: &str| action.input[field].as_str().unwrap_or_default();
                let own = match action.name.as_str() {
                    "delegate" => Some(self.subagents.start(
                        named("name"),
                        named("task"),
                        self.provider.clone(),
                        self.host.clone(),
                    )),
                    "list_subagents" => Some(self.subagents.list()),
                    "stop_subagent" => Some(self.subagents.stop(named("name"))),
                    _ => None,
                };
                if let Some(text) = own {
                    self.store.append(&outcome(text))?;
                    continue;
                }
                let review = match self.review_routine(action) {
                    Some(review) => review,
                    None => self.host.review(action),
                };
                let allowed = match review {
                    Review::Refuse(problem) => Err(problem),
                    Review::Free => Ok(()),
                    Review::Ask(approval) => match channel.ask(&approval).await {
                        Ok(true) => Ok(()),
                        Ok(false) => {
                            Err("The Owner denied this action. It was not carried out.".into())
                        }
                        Err(_) => return Ok(false),
                    },
                };
                let text = match allowed {
                    Err(why_not) => why_not,
                    Ok(()) if self.review_routine(action).is_some() => {
                        self.carry_out_routine(action)
                    }
                    Ok(()) => {
                        let acting = self.host.carry_out(action);
                        match attending(channel, acting, &mut said, false).await {
                            Attended::Done(text) => text,
                            Attended::Stopped | Attended::Interrupted => {
                                stopped = true;
                                left_out = Some(
                                    "The Owner stopped the work: this action was not carried out.",
                                );
                                "The Owner stopped this action before it finished.".into()
                            }
                        }
                    }
                };
                self.store.append(&outcome(self.sized(text)))?;
                if !said.is_empty() && left_out.is_none() {
                    left_out =
                        Some("Your Owner wrote to you meanwhile: this action was not carried out.");
                }
            }
            if stopped {
                return self.stopped(channel).await;
            }
            for text in said {
                self.store.append(&Message::from_owner(text))?;
            }
        }
        let stopped = format!(
            "I stopped after {STEPS_BEFORE_STOPPING} steps without finishing. \
             Tell me whether to go on."
        );
        self.store.append(&Message::from_agent(stopped.clone()))?;
        Ok(channel.send(&stopped).await.is_ok())
    }
}

impl<P: Provider + 'static> Agent<P> {
    /// Keeps what the model is sent within the budget: when the Conversation
    /// has outgrown it, its oldest part is summarised and no longer shown.
    async fn fit(
        &mut self,
        channel: &mut impl Channel,
        actions: &[ActionSpec],
    ) -> Result<(), StoreError> {
        let offered: usize = actions
            .iter()
            .map(|action| weight(&(&action.name, &action.description, &action.input)))
            .sum();
        // The summary must leave room for what comes after it.
        for _ in 0..3 {
            let shown = self.store.shown()?;
            let fixed = self.instructions().len() / 4 + offered + SUMMARISED_NOTE.len() / 4;
            let sizes: Vec<usize> = shown.iter().map(|(_, message)| weight(message)).collect();
            if fixed + sizes.iter().sum::<usize>() <= self.context_budget {
                return Ok(());
            }
            // Keep the most recent entries that fit in half of what is left,
            // starting where an exchange starts so that no Action is parted
            // from its Outcome.
            let room = self.context_budget.saturating_sub(fixed) / 2;
            let starts_exchange = |index: usize| matches!(shown[index].1, Message::Owner(_));
            let starts_step = |index: usize| !matches!(shown[index].1, Message::Outcome { .. });
            let fits = |index: usize| sizes[index..].iter().sum::<usize>() <= room;
            let candidates = || (1..shown.len()).filter(|&index| fits(index));
            let Some(keep_from) = candidates()
                .find(|&index| starts_exchange(index))
                .or_else(|| candidates().find(|&index| starts_step(index)))
                .or_else(|| (1..shown.len()).rev().find(|&index| starts_step(index)))
            else {
                return Ok(());
            };
            let (left_out, _) = shown.split_at(keep_from);
            let through = left_out[left_out.len() - 1].0;
            let summary = self.summarise(channel, left_out).await;
            self.store.summarise_through(through, summary.as_deref())?;
        }
        Ok(())
    }

    /// A summary of the earlier summary and `left_out` together; `None` when
    /// the Provider could not write one.
    async fn summarise(
        &self,
        channel: &mut impl Channel,
        left_out: &[(i64, Message)],
    ) -> Option<String> {
        // Both the material and the summary are bounded by the budget.
        let material_limit = self.context_budget * 3;
        let summary_limit = self.context_budget / 2;
        let mut material = String::new();
        if let Ok(Some(earlier)) = self.store.summary() {
            material.push_str(&format!("Summary so far:\n{earlier}\n\nWhat was said since:\n"));
        }
        for (_, message) in left_out {
            let line = match message {
                Message::Owner(text) => format!("Owner: {text}"),
                Message::Agent { text, actions } => {
                    let acted: Vec<String> = actions
                        .iter()
                        .map(|action| format!("[{} {}]", action.name, action.input))
                        .collect();
                    format!("Agent: {text} {}", acted.join(" "))
                }
                Message::Outcome { text, .. } => format!("Result: {text}"),
                Message::Notice(text) => format!("Notice: {text}"),
            };
            material.push_str(clip(&line, 800));
            material.push('\n');
        }
        let material = clip(&material, material_limit).to_owned();
        let conversation = [Message::Owner(material)];
        let request = Request {
            instructions: SUMMARY_INSTRUCTIONS,
            conversation: &conversation,
            actions: &[],
        };
        let reply = showing_work(channel, patiently(&*self.provider, request)).await.ok()?;
        Some(clip(reply.text.trim(), summary_limit).to_owned())
    }

    /// What was said that matches the query of a `search_conversation` request.
    fn search(&self, request: &ActionRequest) -> String {
        let Some(query) = request.input["query"].as_str() else {
            return "The input of search_conversation is not valid: missing field `query`.".into();
        };
        match self.store.search(query) {
            Err(error) => explain_storage(&error),
            Ok(found) if found.is_empty() => {
                format!("Nothing in the conversation matches \"{query}\".")
            }
            Ok(found) => found
                .iter()
                .map(|(date, message)| match message {
                    Message::Owner(text) => format!("{date} Owner: {}", clip(text, 600)),
                    Message::Agent { text, .. } => format!("{date} Agent: {}", clip(text, 600)),
                    Message::Outcome { text, .. } | Message::Notice(text) => {
                        format!("{date} {}", clip(text, 600))
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// The estimated size of `value` in tokens, once written out for a Provider.
fn weight(value: &impl serde::Serialize) -> usize {
    serde_json::to_string(value).map_or(0, |written| written.len()) / 4
}

/// The start of `text` that fits in `limit` bytes.
pub(crate) fn clip(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn search_spec() -> ActionSpec {
    ActionSpec {
        name: "search_conversation".into(),
        description: "Finds the exact words of something said earlier in your conversation with \
                      your Owner, however long ago, including what is now only summarised."
            .into(),
        input: serde_json::json!({
            "type": "object",
            "properties": { "query": { "type": "string", "description": "A few words to look for." } },
            "required": ["query"],
        }),
    }
}

/// Asks the Provider, waiting and asking again while it is only busy.
pub(crate) async fn patiently(
    provider: &impl Provider,
    request: Request<'_>,
) -> Result<crate::Reply, ProviderError> {
    let mut wait = Duration::from_secs(1);
    for _ in 1..TRIES_WHILE_BUSY {
        match provider.reply(request).await {
            Err(ProviderError::Busy { retry_after, .. })
                if retry_after.is_none_or(|asked| asked <= LONGEST_WAIT) =>
            {
                tokio::time::sleep(retry_after.unwrap_or(wait)).await;
                wait = (wait * 2).min(LONGEST_WAIT);
            }
            outcome => return outcome,
        }
    }
    provider.reply(request).await
}

impl<P: Provider + 'static> Agent<P> {
    /// `outcome`, cut when it alone would take too much of the budget: no
    /// later summary could make room for it.
    fn sized(&self, outcome: String) -> String {
        // In bytes: a quarter of the budget, at four characters to a token.
        let limit = self.context_budget;
        if outcome.len() <= limit {
            return outcome;
        }
        let kept = clip(&outcome, limit);
        format!("{kept}\n[cut to fit: {} more characters]", outcome.len() - kept.len())
    }

    /// Tells the Owner, and the model for later, that the work was stopped.
    async fn stopped(&mut self, channel: &mut impl Channel) -> Result<bool, StoreError> {
        self.store.append(&Message::from_agent("Stopped."))?;
        Ok(channel.send("Stopped.").await.is_ok())
    }
}

impl<P: Provider + 'static> Agent<P> {
    /// Tells the model what a Sub-agent or a Routine run ended with, and
    /// keeps a Routine's result with it.
    fn note(&mut self, report: Report) -> Result<(), StoreError> {
        if let Some(name) = &report.routine {
            let routines = self.store.routines()?;
            if let Some(mut routine) = routines.into_iter().find(|routine| &routine.name == name) {
                routine.last_result = Some(one_line(clip(&report.ended, 300)));
                self.store.save_routine(&routine)?;
            }
        }
        self.store.append(&Message::Notice(format!("{} {}", report.who, report.ended)))
    }

    /// Starts every Routine whose time has come, and moves each to its next.
    fn run_due_routines(&mut self) -> Result<(), StoreError> {
        let now = (self.clock)();
        for mut routine in self.store.routines()? {
            if routine.paused || routine.next_run > now.timestamp() {
                continue;
            }
            // A single reminder goes straight to the Agent, who has the whole
            // Conversation to make sense of it: nothing to work out afresh.
            if routine.cron.is_none() {
                self.store.remove_routine(&routine.name)?;
                self.store.append(&Message::Notice(format!(
                    "It is time for \"{}\", which you scheduled: {}",
                    routine.name, routine.instruction
                )))?;
                self.owed = true;
                continue;
            }
            let due = routines::written(routine.next_run, now);
            // Orbite was stopped, or busy, when it should have run.
            let late = now.timestamp() - routine.next_run > LATE_AFTER.as_secs() as i64;
            let who = if late {
                format!("Routine \"{}\" (run late: it was due at {due})", routine.name)
            } else {
                format!("Routine \"{}\"", routine.name)
            };
            let task = format!(
                "{}\n\n(This is a run of the routine \"{}\". It is now {}.)",
                routine.instruction,
                routine.name,
                now.format(routines::MOMENT)
            );
            let started = self.subagents.run_routine(
                &routine.name,
                who,
                &task,
                self.provider.clone(),
                self.host.clone(),
            );
            if !started {
                // Too much is going on: try again at the next look at the time.
                continue;
            }
            // However many occurrences were missed, it runs once and moves on.
            match routine.cron.as_deref().map(|cron| routines::next_after(cron, now)) {
                Some(Ok(next)) => {
                    routine.next_run = next.timestamp();
                    routine.last_run = Some(now.timestamp());
                    self.store.save_routine(&routine)?;
                }
                // It ran its one time, or can never come up again.
                None | Some(Err(_)) => {
                    self.store.remove_routine(&routine.name)?;
                }
            }
        }
        Ok(())
    }

    /// The Routine a `schedule_routine` request describes, or what is wrong with it.
    fn planned(&self, request: &ActionRequest) -> Result<Routine, String> {
        let now = (self.clock)();
        let field = |name: &str| {
            request.input[name].as_str().map(str::trim).filter(|text| !text.is_empty())
        };
        let name = slug(field("name").unwrap_or_default())
            .map_err(|error| format!("The routine cannot be named that: {error}."))?;
        let instruction = field("instruction").ok_or("A routine needs an instruction.")?.to_owned();
        let (cron, next) = match (field("cron"), field("at")) {
            (Some(_), Some(_)) => return Err("Give `cron` or `at`, not both.".into()),
            (None, None) => {
                return Err("A routine needs `cron` (to recur) or `at` (to run once).".into());
            }
            (Some(cron), None) => (Some(cron.to_owned()), routines::next_after(cron, now)?),
            (None, Some(at)) => (None, routines::moment(at, now)?),
        };
        Ok(Routine {
            name,
            instruction,
            cron,
            next_run: next.timestamp(),
            paused: false,
            last_run: None,
            last_result: None,
        })
    }

    /// The Routine a request names, or what to tell the model when there is none.
    fn routine_named(&self, request: &ActionRequest) -> Result<Routine, String> {
        let asked = request.input["name"].as_str().unwrap_or_default();
        let name = slug(asked).unwrap_or_default();
        let routines = self.store.routines().map_err(|error| explain_storage(&error))?;
        let names: Vec<String> = routines.iter().map(|routine| routine.name.clone()).collect();
        routines.into_iter().find(|routine| routine.name == name).ok_or_else(|| {
            format!(
                "There is no routine called \"{asked}\". The routines are: {}.",
                names.join(", ")
            )
        })
    }

    /// What the gate decides about an Action on Routines; `None` for any
    /// other Action.
    fn review_routine(&self, request: &ActionRequest) -> Option<Review> {
        let now = (self.clock)();
        Some(match request.name.as_str() {
            "list_routines" => Review::Free,
            "schedule_routine" => match self.planned(request) {
                Err(problem) => Review::Refuse(problem),
                // A single reminder is not worth interrupting the Owner for.
                Ok(Routine { cron: None, .. }) => Review::Free,
                Ok(routine) => {
                    let cron = routine.cron.as_deref().unwrap_or_default();
                    let replaces = self
                        .store
                        .routines()
                        .is_ok_and(|all| all.iter().any(|other| other.name == routine.name));
                    Review::Ask(Approval {
                        summary: if replaces { "Replace a routine" } else { "Schedule a routine" }
                            .into(),
                        detail: format!(
                            "{}\n\n{}\n\nSchedule: {cron} ({})\nNext run: {}",
                            routine.name,
                            routine.instruction,
                            routines::in_words(cron),
                            routines::written(routine.next_run, now)
                        ),
                        irreversible: replaces,
                    })
                }
            },
            "pause_routine" | "resume_routine" | "run_routine" => match self.routine_named(request)
            {
                Err(problem) => Review::Refuse(problem),
                Ok(_) => Review::Free,
            },
            "remove_routine" => match self.routine_named(request) {
                Err(problem) => Review::Refuse(problem),
                Ok(routine) => Review::Ask(Approval {
                    summary: "Remove a routine".into(),
                    detail: format!("{}\n\n{}", routine.name, routine.instruction),
                    irreversible: true,
                }),
            },
            _ => return None,
        })
    }

    /// Carries out an Action on Routines the gate let through.
    fn carry_out_routine(&mut self, request: &ActionRequest) -> String {
        let now = (self.clock)();
        let done = match request.name.as_str() {
            "list_routines" => self.store.routines().map(|routines| {
                if routines.is_empty() {
                    return "No routine is scheduled.".to_owned();
                }
                routines.iter().map(|routine| describe(routine, now)).collect::<Vec<_>>().join("\n")
            }),
            "schedule_routine" => match self.planned(request) {
                Err(problem) => return problem,
                Ok(routine) => self.store.save_routine(&routine).map(|()| {
                    format!(
                        "Scheduled \"{}\". Next run: {}.",
                        routine.name,
                        routines::written(routine.next_run, now)
                    )
                }),
            },
            _ => match self.routine_named(request) {
                Err(problem) => return problem,
                Ok(mut routine) => match request.name.as_str() {
                    "pause_routine" => {
                        routine.paused = true;
                        self.store
                            .save_routine(&routine)
                            .map(|()| format!("Paused \"{}\".", routine.name))
                    }
                    "resume_routine" => {
                        routine.paused = false;
                        // What was missed while paused is not made up.
                        if let Some(Ok(next)) =
                            routine.cron.as_deref().map(|cron| routines::next_after(cron, now))
                        {
                            routine.next_run = next.timestamp();
                        }
                        self.store.save_routine(&routine).map(|()| {
                            format!(
                                "Resumed \"{}\". Next run: {}.",
                                routine.name,
                                routines::written(routine.next_run, now)
                            )
                        })
                    }
                    "remove_routine" => self
                        .store
                        .remove_routine(&routine.name)
                        .map(|_| format!("Removed the routine \"{}\".", routine.name)),
                    _ => {
                        let who = format!("Routine \"{}\" (tried out now)", routine.name);
                        let task = format!(
                            "{}\n\n(This is a run of the routine \"{}\". It is now {}.)",
                            routine.instruction,
                            routine.name,
                            now.format(routines::MOMENT)
                        );
                        let started = self.subagents.run_routine(
                            &routine.name,
                            who,
                            &task,
                            self.provider.clone(),
                            self.host.clone(),
                        );
                        Ok(if started {
                            format!(
                                "Started a run of \"{}\" now. Like every run, it can look at \
                                 things but changes nothing by itself; its report will reach \
                                 you as a notice. Its schedule is unchanged.",
                                routine.name
                            )
                        } else {
                            "It could not start now: a run of it is still going, or too much is."
                                .into()
                        })
                    }
                },
            },
        };
        done.unwrap_or_else(|error| explain_storage(&error))
    }
}

/// A Routine as listed to the model.
fn describe(routine: &Routine, now: DateTime<FixedOffset>) -> String {
    let schedule = routine.cron.as_deref().unwrap_or("once");
    let next = if routine.paused {
        "paused".to_owned()
    } else {
        format!("next: {}", routines::written(routine.next_run, now))
    };
    let last = match (routine.last_run, &routine.last_result) {
        (Some(at), Some(result)) => format!("last: {} {result}", routines::written(at, now)),
        (Some(at), None) => format!("last: {} (still running)", routines::written(at, now)),
        _ => "never run".to_owned(),
    };
    format!(
        "- {} [{schedule}] {next} · {last} · instruction: {}",
        routine.name, routine.instruction
    )
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How work ended while the Owner could interrupt it.
enum Attended<T> {
    Done(T),
    /// The Owner said to stop; the work was abandoned.
    Stopped,
    /// The Owner said something else and the work was abandoned for it.
    Interrupted,
}

/// Runs `work` while showing the Owner that the Agent is busy and listening
/// to them. "Stop" abandons the work. Anything else is added to `said`, and
/// abandons the work too when `yields` is set; otherwise the work goes on.
async fn attending<T>(
    channel: &mut impl Channel,
    work: impl Future<Output = T>,
    said: &mut Vec<String>,
    yields: bool,
) -> Attended<T> {
    tokio::pin!(work);
    let mut listening = true;
    loop {
        channel.working().await;
        tokio::select! {
            done = &mut work => return Attended::Done(done),
            written = channel.interruption(), if listening => match written {
                // The Channel closed: the work still gets to finish.
                None => listening = false,
                Some(text) if means_stop(&text) => return Attended::Stopped,
                Some(text) => {
                    said.push(text);
                    if yields {
                        return Attended::Interrupted;
                    }
                }
            },
            () = tokio::time::sleep(WORKING_SIGNAL_EVERY) => {}
        }
    }
}

/// True when what the Owner wrote is the order to stop.
fn means_stop(text: &str) -> bool {
    let word = text.trim().trim_end_matches(['!', '.']).to_lowercase();
    word == "stop" || word == "/stop"
}

/// Runs `work` to its end while showing the Owner that the Agent is busy.
async fn showing_work<T>(channel: &mut impl Channel, work: impl Future<Output = T>) -> T {
    tokio::pin!(work);
    loop {
        channel.working().await;
        tokio::select! {
            done = &mut work => break done,
            () = tokio::time::sleep(WORKING_SIGNAL_EVERY) => {}
        }
    }
}

fn explain_storage(error: &StoreError) -> String {
    format!("I couldn't use my own storage ({error}).")
}

/// A Provider failure, in words for the Owner.
pub(crate) fn explain(error: &ProviderError) -> String {
    match error {
        ProviderError::Rejected(reason) => {
            format!(
                "My AI provider refused my access ({reason}). Check the API key, then try again."
            )
        }
        ProviderError::Unreachable(reason) => {
            format!("I couldn't reach my AI provider ({reason}). Try again in a moment.")
        }
        ProviderError::Busy { reason, .. } => {
            format!("My AI provider is too busy to answer ({reason}). Try again later.")
        }
        ProviderError::Failed(reason) => {
            format!("My AI provider failed to answer ({reason}). Try again in a moment.")
        }
    }
}
