use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Local, NaiveTime};
use tokio::sync::mpsc;

use crate::actions::{Host, Review};
use crate::attachments::{attachments, is_showable_image};
use crate::memory::Memory;
use crate::routines::{self, Routine};
use crate::skills::Skills;
use crate::slug::slug;
use crate::subagents::{Report, Subagents};
use crate::web::Web;
use crate::{
    ActionRequest, ActionSpec, Answer, Approval, Channel, Message, Provider, ProviderError,
    Request, Step, Store, StoreError,
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

When your Owner sends a file, their message ends with a line such as \
[attached: /path/to/file]: the file is on this machine, for you to read or \
use. Images sent lately are also shown to you directly.

A notice is something you are told that your Owner did not write, such as \
the report of a sub-agent or of a routine's run: your Owner has not seen it, so tell \
them what matters in it. When a notice calls for no word to your Owner, reply \
with exactly [silent]. What you write on your own initiative during your \
Owner's quiet hours is kept until they end; start it with [urgent] only when \
it truly cannot wait.

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

/// An image is shown to the model while the message it came with is among
/// this many latest entries of the Conversation.
const ENTRIES_SHOWING_IMAGES: usize = 6;

/// Added, for the model only, to a message whose image it cannot be shown.
const BLIND_NOTE: &str = "\n[An image came with this, but the model you run on cannot see images: \
tell your Owner you cannot look at it.]";

/// How many of the latest entries of the Conversation a Wake-up is shown.
const WAKE_UP_SEES: usize = 12;

const WAKE_UP_INSTRUCTIONS: &str = "\
You are a personal AI agent waking up by yourself, with nobody asking, to \
review what is pending with your Owner. You are shown what you remember, what \
is already scheduled, and the end of your conversation. Is there something \
that should happen now and will not happen by itself: a promise of yours \
coming due, a follow-up your Owner is waiting for, something they said they \
had to do and may have forgotten, a habit you could offer to take over? What \
is already scheduled takes care of itself, and what you have already brought \
up and your Owner has not answered must not be brought up again: once is \
enough, they will come back to it. If nothing calls for action now, \
which is the usual case, reply with exactly [silent]. Otherwise say in one or \
two sentences what should be done and why now.";

/// The whole answer to a notice that calls for no word to the Owner.
const SILENT: &str = "[silent]";

/// Starts what cannot wait for the end of the Owner's quiet hours.
const URGENT: &str = "[urgent]";

/// A Routine started this long after it was due is announced as late.
const LATE_AFTER: Duration = Duration::from_secs(300);

/// How many times a busy Provider is asked before the Owner is told.
const TRIES_WHILE_BUSY: usize = 7;

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
    /// How often the Agent wakes up when its Owner has set no pace, in
    /// minutes; zero for never.
    wake_up_pace: u32,
    /// When the Owner last wrote or the Agent last woke up, in seconds since 1970.
    last_active: i64,
    /// False once the model has turned out not to take images.
    sees_images: bool,
}

impl<P: Provider + 'static> Agent<P> {
    pub fn new(provider: P, store: Store) -> Self {
        let host = Host {
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            command_timeout: Duration::from_secs(120),
            memory: None,
            skills: None,
            web: Web::new(),
            busy_wait: Duration::from_secs(1),
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
            wake_up_pace: 0,
            last_active: 0,
            sees_images: true,
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

    /// How often the Agent wakes up by itself to review what is pending,
    /// unless its Owner has set another pace; zero for never.
    pub fn waking_up_every_minutes(mut self, minutes: u32) -> Self {
        self.wake_up_pace = minutes;
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

    /// How long a busy Provider is first waited for; each further wait is
    /// twice the one before.
    pub fn waiting_for_a_busy_provider(mut self, first_wait: Duration) -> Self {
        self.host_mut().busy_wait = first_wait;
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
        if self.unfinished().unwrap_or(false) && !self.respond_on(channel, false).await {
            return;
        }
        self.last_active = (self.clock)().timestamp();
        let mut time_check = tokio::time::interval(self.time_check);
        time_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let text = tokio::select! {
                written = channel.receive() => match written {
                    Some(text) => text,
                    None => return,
                },
                Some(report) = self.reports.recv() => {
                    if self.note(report).is_ok() && !self.respond_on(channel, true).await {
                        return;
                    }
                    continue;
                }
                _ = time_check.tick() => {
                    let _ = self.run_due_routines();
                    if std::mem::take(&mut self.owed) && !self.respond_on(channel, true).await {
                        return;
                    }
                    if !self.quiet_now() && !self.release_held(channel).await {
                        return;
                    }
                    if self.wake_up_is_due() {
                        self.wake_up().await;
                        if std::mem::take(&mut self.owed) && !self.respond_on(channel, true).await {
                            return;
                        }
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
            self.last_active = (self.clock)().timestamp();
            // The Owner is there after all: what was kept for later is due.
            if !self.release_held(channel).await {
                return;
            }
            if let Err(error) = self.store.append(&Message::from_owner(text)) {
                if channel.send(&explain_storage(&error)).await.is_err() {
                    return;
                }
                continue;
            }
            if !self.respond_on(channel, false).await {
                return;
            }
        }
    }

    /// Who the Agent is, what it remembers and what it knows how to do, as
    /// told to the model.
    fn instructions(&self) -> String {
        let mut instructions = INSTRUCTIONS.to_owned();
        let level = self.level();
        instructions.push_str(&format!(
            "\n\nYour permission level is {}: {}",
            level.name(),
            level
                .meaning()
                .replace("the Agent", "you")
                .replace(" it always asks", " you always ask")
        ));
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
        if let Some((from, until)) = self.quiet_hours() {
            let now = if self.quiet_now() {
                "it is quiet hours now"
            } else {
                "it is not quiet hours now"
            };
            instructions
                .push_str(&format!(" Your Owner's quiet hours are {from} to {until}: {now}."));
        }
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
    async fn respond_on(&mut self, channel: &mut impl Channel, initiated: bool) -> bool {
        let open = match self.respond(channel, initiated).await {
            Ok(open) => open,
            Err(error) => channel.send(&explain_storage(&error)).await.is_ok(),
        };
        channel.idle().await;
        open
    }

    /// Thinks and acts, turn after turn, until the model has nothing left to
    /// do. `initiated` is true when the Agent acts on its own initiative
    /// rather than in answer to its Owner.
    async fn respond(
        &mut self,
        channel: &mut impl Channel,
        initiated: bool,
    ) -> Result<bool, StoreError> {
        let mut actions = self.host.specs();
        actions.push(search_spec());
        actions.push(send_file_spec());
        actions.extend(Subagents::specs());
        actions.extend(routines::specs());
        actions.push(quiet_hours_spec());
        actions.push(wake_ups_spec());
        actions.extend(permission_specs());
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
            // Images the Owner sent lately are shown to the model; older
            // ones are costly to send again and stay as a path it can use.
            let mut images = Vec::new();
            let recent = conversation.len().saturating_sub(ENTRIES_SHOWING_IMAGES);
            for (place, message) in conversation.iter_mut().enumerate().skip(recent) {
                let Message::Owner(text) = message else { continue };
                let sent: Vec<PathBuf> =
                    attachments(text).filter(|file| is_showable_image(file)).collect();
                if sent.is_empty() {
                    continue;
                }
                if self.sees_images {
                    images.extend(sent.into_iter().map(|file| (place, file)));
                } else {
                    text.push_str(BLIND_NOTE);
                }
            }
            let request = Request {
                instructions: &instructions,
                conversation: &conversation,
                actions: &actions,
                images: &images,
            };
            let mut said = Vec::new();
            let thinking = patiently(&*self.provider, request, self.host.busy_wait);
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
                // Most likely a model that does not take images: go on without.
                Attended::Done(Err(ProviderError::Failed(_))) if !images.is_empty() => {
                    self.sees_images = false;
                    continue;
                }
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
            if !reply.text.is_empty() && !self.deliver(channel, &reply.text, initiated).await? {
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
                if action.name == "send_file" {
                    let text = match self.send_file(channel, action).await {
                        Some(text) => text,
                        None => return Ok(false),
                    };
                    self.store.append(&outcome(text))?;
                    continue;
                }
                if action.name == "search_conversation" {
                    channel.doing(&Step::of(action)).await;
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
                    "set_quiet_hours" => Some(self.set_quiet_hours(named("from"), named("until"))),
                    "set_wake_ups" => Some(self.set_wake_ups(&action.input["every_minutes"])),
                    _ => None,
                };
                if let Some(text) = own {
                    channel.doing(&Step::of(action)).await;
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
                    Review::Ask(approval) if self.already_allowed(&approval, action) => Ok(()),
                    // On its own initiative the Agent does not disturb its Owner's rest.
                    Review::Ask(_) if initiated && self.quiet_now() => Err(format!(
                        "This needs your Owner's approval, and it is their quiet hours ({}): it \
                         was not asked for and not carried out. If it still matters, schedule it \
                         for after they end.",
                        self.quiet_hours()
                            .map(|(from, until)| format!("{from} to {until}"))
                            .unwrap_or_default()
                    )),
                    Review::Ask(approval) => match channel.ask(&approval).await {
                        Ok(Answer::Allowed) => Ok(()),
                        Ok(Answer::AlwaysAllowed) => {
                            // A Critical Action is only ever allowed the once.
                            if !approval.critical {
                                self.always_allow(action)?;
                            }
                            Ok(())
                        }
                        Ok(Answer::Denied) => {
                            Err("The Owner denied this action. It was not carried out.".into())
                        }
                        Ok(Answer::Wrote(text)) if means_stop(&text) => {
                            stopped = true;
                            left_out = Some(
                                "The Owner stopped the work: this action was not carried out.",
                            );
                            Err("The Owner stopped the work: this action was not carried out."
                                .into())
                        }
                        Ok(Answer::Wrote(text)) => {
                            said.push(text);
                            Err("Your Owner wrote to you instead of answering: this action was \
                                 not carried out."
                                .into())
                        }
                        Err(_) => return Ok(false),
                    },
                };
                if allowed.is_ok() {
                    channel.doing(&Step::of(action)).await;
                }
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
            images: &[],
        };
        let reply = showing_work(channel, patiently(&*self.provider, request, self.host.busy_wait))
            .await
            .ok()?;
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

fn permission_specs() -> Vec<ActionSpec> {
    let spec = |name: &str, description: &str, properties: serde_json::Value| {
        let required: Vec<&String> = properties.as_object().unwrap().keys().collect();
        ActionSpec {
            name: name.into(),
            description: description.into(),
            input: serde_json::json!({ "type": "object", "properties": properties, "required": required }),
        }
    };
    vec![
        spec(
            "set_permission_level",
            "Changes how much you do without asking your Owner, only when they ask you to. \
             Loosening it needs their approval.",
            serde_json::json!({ "level": { "type": "string", "enum": ["careful", "balanced", "free"] } }),
        ),
        spec(
            "list_always_allowed",
            "Lists what your Owner allowed once and for all.",
            serde_json::json!({}),
        ),
        spec(
            "revoke_always_allowed",
            "Takes back something your Owner allowed once and for all.",
            serde_json::json!({ "what": { "type": "string", "description": "Words of the entry to take back." } }),
        ),
    ]
}

fn wake_ups_spec() -> ActionSpec {
    ActionSpec {
        name: "set_wake_ups".into(),
        description: "Sets how often you, the agent, review by yourself what is pending with \
                      your Owner when nothing else is going on (your own check-ins, not an \
                      alarm or reminder for your Owner: use schedule_routine for those). 0 \
                      turns them off."
            .into(),
        input: serde_json::json!({
            "type": "object",
            "properties": { "every_minutes": { "type": "integer", "description": "Minutes between wake-ups; 0 for never." } },
            "required": ["every_minutes"],
        }),
    }
}

fn quiet_hours_spec() -> ActionSpec {
    ActionSpec {
        name: "set_quiet_hours".into(),
        description: "Sets the hours during which you do not write to your Owner on your own \
                      initiative; what you would have said is delivered when they end. Give \
                      both times empty to turn them off."
            .into(),
        input: serde_json::json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "When they start, as HH:MM in your Owner's time." },
                "until": { "type": "string", "description": "When they end, as HH:MM in your Owner's time." },
            },
            "required": ["from", "until"],
        }),
    }
}

fn send_file_spec() -> ActionSpec {
    ActionSpec {
        name: "send_file".into(),
        description: "Sends your Owner a file from this machine, in the conversation.".into(),
        input: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute, or relative to your workspace folder." },
                "caption": { "type": "string", "description": "A few words to go with it." },
            },
            "required": ["path"],
        }),
    }
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
    first_wait: Duration,
) -> Result<crate::Reply, ProviderError> {
    // Each wait is at least twice the one before: a limit counted over a
    // minute is not lifted by the few seconds a Provider may ask for.
    let mut wait = first_wait;
    for _ in 1..TRIES_WHILE_BUSY {
        match provider.reply(request).await {
            Err(ProviderError::Busy { retry_after, .. })
                if retry_after.is_none_or(|asked| asked <= LONGEST_WAIT) =>
            {
                tokio::time::sleep(retry_after.unwrap_or_default().max(wait)).await;
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
    /// Says `text` to the Owner. What the Agent says on its own initiative
    /// may be left unsaid, or kept until the Owner's quiet hours end. False
    /// once the Channel is closed.
    async fn deliver(
        &mut self,
        channel: &mut impl Channel,
        text: &str,
        initiated: bool,
    ) -> Result<bool, StoreError> {
        // The marks are for Orbite: whatever the model does with them, the
        // Owner never sees one.
        let urgent = text.contains(URGENT);
        let silent = text.trim() == SILENT;
        let said = text.replace(SILENT, "").replace(URGENT, "");
        let text = said.trim();
        if !initiated {
            // Asked directly, the Agent owes an answer.
            let text = if text.is_empty() { "(nothing to add)" } else { text };
            return Ok(channel.send(text).await.is_ok());
        }
        if text.is_empty() || silent {
            return Ok(true);
        }
        if !urgent && self.quiet_now() {
            let mut held = self.held()?;
            held.push(text.to_owned());
            let kept = serde_json::to_string(&held).unwrap_or_default();
            self.store.set_setting("held", Some(&kept))?;
            return Ok(true);
        }
        Ok(channel.send(text).await.is_ok())
    }

    /// What was kept for after the Owner's quiet hours.
    fn held(&self) -> Result<Vec<String>, StoreError> {
        let kept = self.store.setting("held")?;
        Ok(kept.and_then(|kept| serde_json::from_str(&kept).ok()).unwrap_or_default())
    }

    /// Says what was kept for later, all at once. False once the Channel is closed.
    async fn release_held(&mut self, channel: &mut impl Channel) -> bool {
        let Ok(held) = self.held() else { return true };
        if held.is_empty() {
            return true;
        }
        if channel.send(&held.join("\n\n")).await.is_err() {
            return false;
        }
        let _ = self.store.set_setting("held", None);
        true
    }

    /// How often the Agent wakes up, in minutes; zero for never.
    fn wake_up_minutes(&self) -> u32 {
        let set = self.store.setting("wake_up_minutes").ok().flatten();
        set.and_then(|minutes| minutes.parse().ok()).unwrap_or(self.wake_up_pace)
    }

    fn set_wake_ups(&mut self, every_minutes: &serde_json::Value) -> String {
        let Some(minutes) = every_minutes.as_u64().filter(|minutes| *minutes <= 7 * 24 * 60) else {
            return "Give `every_minutes` as a number of minutes, or 0 to turn wake-ups off."
                .into();
        };
        match self.store.set_setting("wake_up_minutes", Some(&minutes.to_string())) {
            Err(error) => explain_storage(&error),
            Ok(()) if minutes == 0 => "Wake-ups are off.".into(),
            Ok(()) => {
                format!("I now wake up every {minutes} minutes when nothing else is going on.")
            }
        }
    }

    /// True when it is time for the Agent to review what is pending.
    fn wake_up_is_due(&self) -> bool {
        let minutes = i64::from(self.wake_up_minutes());
        minutes > 0
            && !self.quiet_now()
            && (self.clock)().timestamp() - self.last_active >= minutes * 60
    }

    /// Reviews what is pending, on little: the model is shown only the end of
    /// the Conversation and decides whether anything is worth bringing up.
    /// What it brings up becomes a notice; nothing else leaves a trace.
    async fn wake_up(&mut self) {
        let now = (self.clock)();
        self.last_active = now.timestamp();
        let mut material = String::new();
        if let Ok(Some(summary)) = self.store.summary() {
            material.push_str(&format!("Earlier: {}\n\n", clip(&summary, 1_200)));
        }
        if let Some(memory) = &self.host.memory {
            let entries: Vec<String> =
                memory.entries().iter().map(|(name, text)| format!("{name}: {text}")).collect();
            if !entries.is_empty() {
                material.push_str(&format!(
                    "What you remember: {}\n\n",
                    clip(&entries.join("; "), 1_200)
                ));
            }
        }
        if let Ok(routines) = self.store.routines()
            && !routines.is_empty()
        {
            let scheduled: Vec<String> = routines
                .iter()
                .map(|routine| clip(&describe(routine, now), 200).to_owned())
                .collect();
            material.push_str(&format!("Already scheduled:\n{}\n\n", scheduled.join("\n")));
        }
        material.push_str("The end of your conversation with your Owner:\n");
        let conversation = self.store.conversation().unwrap_or_default();
        for message in &conversation[conversation.len().saturating_sub(WAKE_UP_SEES)..] {
            let line = match message {
                Message::Owner(text) => format!("Owner: {text}"),
                Message::Agent { text, .. } if text.is_empty() => continue,
                Message::Agent { text, .. } => format!("You: {text}"),
                Message::Outcome { text, .. } => format!("Result: {text}"),
                Message::Notice(text) => format!("Notice: {text}"),
            };
            material.push_str(clip(&line, 300));
            material.push('\n');
        }
        let instructions =
            format!("{WAKE_UP_INSTRUCTIONS}\n\nIt is {}.", now.format("%A %Y-%m-%d %H:%M"));
        let asked = [Message::Owner(material)];
        let request = Request {
            instructions: &instructions,
            conversation: &asked,
            actions: &[],
            images: &[],
        };
        let Ok(reply) = patiently(&*self.provider, request, self.host.busy_wait).await else {
            return;
        };
        let thought = reply.text.trim();
        if thought.is_empty() || thought.contains(SILENT) {
            return;
        }
        let noted = self.store.append(&Message::Notice(format!(
            "Waking up by yourself to review what is pending, you noted this, which your Owner \
             has not seen: {thought}"
        )));
        self.owed = noted.is_ok();
    }

    /// The Owner's quiet hours as (from, until), each written HH:MM.
    fn quiet_hours(&self) -> Option<(String, String)> {
        let kept = self.store.setting("quiet_hours").ok()??;
        let (from, until) = kept.split_once('-')?;
        Some((from.to_owned(), until.to_owned()))
    }

    /// True during the Owner's quiet hours.
    fn quiet_now(&self) -> bool {
        let Some((from, until)) = self.quiet_hours() else { return false };
        let (Ok(from), Ok(until)) =
            (NaiveTime::parse_from_str(&from, "%H:%M"), NaiveTime::parse_from_str(&until, "%H:%M"))
        else {
            return false;
        };
        let now = (self.clock)().time();
        if from <= until { from <= now && now < until } else { now >= from || now < until }
    }

    fn set_quiet_hours(&mut self, from: &str, until: &str) -> String {
        let (from, until) = (from.trim(), until.trim());
        if from.is_empty() && until.is_empty() {
            return match self.store.set_setting("quiet_hours", None) {
                Ok(()) => "Quiet hours are off.".into(),
                Err(error) => explain_storage(&error),
            };
        }
        let read = |time: &str| {
            NaiveTime::parse_from_str(time, "%H:%M").map(|time| time.format("%H:%M").to_string())
        };
        let (Ok(from), Ok(until)) = (read(from), read(until)) else {
            return "Give both times as HH:MM, or both empty to turn quiet hours off.".into();
        };
        match self.store.set_setting("quiet_hours", Some(&format!("{from}-{until}"))) {
            Ok(()) => format!("Quiet hours are now {from} to {until}."),
            Err(error) => explain_storage(&error),
        }
    }

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
                    "The moment you scheduled \"{}\" for has come. Your Owner has not seen \
                     this notice and is waiting for nothing: if it is a reminder, give them \
                     the reminder itself now, in your own words; otherwise do what it says. \
                     It says: {}",
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

    /// Sends the Owner the file a `send_file` request names; the text is the
    /// Outcome, or `None` once the Channel is closed.
    async fn send_file(
        &mut self,
        channel: &mut impl Channel,
        request: &ActionRequest,
    ) -> Option<String> {
        let Some(path) = request.input["path"].as_str() else {
            return Some("The input of send_file is not valid: missing field `path`.".into());
        };
        let caption = request.input["caption"].as_str().unwrap_or_default();
        let file = self.host.locate(path);
        if !file.is_file() {
            return Some(format!("There is no file at {path}."));
        }
        // The Owner's own secret going to the Owner is no leak, but it does
        // pass through the Channel's servers: they decide.
        if crate::actions::looks_secret(path) {
            let approval = Approval {
                summary: "Send a file".into(),
                detail: format!(
                    "{path}\n\nThis may be a secret, and it would pass through the Channel's servers."
                ),
                critical: true,
                ..Approval::default()
            };
            match channel.ask(&approval).await {
                Ok(Answer::Allowed | Answer::AlwaysAllowed) => {}
                Ok(_) => return Some("The Owner did not allow this file to be sent.".into()),
                Err(_) => return None,
            }
        }
        channel.doing(&Step::of(request)).await;
        Some(match channel.send_file(&file, caption).await {
            Ok(()) => format!("Sent {path} to your Owner."),
            Err(error) => format!("Could not send {path}: {error}."),
        })
    }

    /// The Owner's Permission Level.
    fn level(&self) -> Level {
        let set = self.store.setting("permission_level").ok().flatten();
        set.and_then(|name| Level::named(&name)).unwrap_or(Level::Careful)
    }

    /// What the Owner has allowed once and for all.
    fn always_allowed(&self) -> Result<Vec<String>, StoreError> {
        let stored = self.store.setting("always_allowed")?;
        Ok(stored.and_then(|stored| serde_json::from_str(&stored).ok()).unwrap_or_default())
    }

    fn always_allow(&mut self, action: &ActionRequest) -> Result<(), StoreError> {
        let mut allowed = self.always_allowed()?;
        let what = covered_by_always(action);
        if !allowed.contains(&what) {
            allowed.push(what);
        }
        self.store.set_setting(
            "always_allowed",
            Some(&serde_json::to_string(&allowed).unwrap_or_default()),
        )
    }

    /// True when an Action that would otherwise be asked about needs no
    /// Approval: the Permission Level lets it through, or the Owner allowed
    /// it once and for all. Never true of a Critical Action.
    fn already_allowed(&self, approval: &Approval, action: &ActionRequest) -> bool {
        if approval.critical {
            return false;
        }
        match self.level() {
            Level::Free => true,
            Level::Balanced if approval.contained => true,
            _ => self
                .always_allowed()
                .is_ok_and(|allowed| allowed.contains(&covered_by_always(action))),
        }
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
            "list_always_allowed" | "revoke_always_allowed" => Review::Free,
            "set_permission_level" => {
                let asked = request.input["level"].as_str().unwrap_or_default();
                match Level::named(asked) {
                    // Refused when carried out, with the levels that exist.
                    None => Review::Free,
                    Some(level) if level <= self.level() => Review::Free,
                    // Nobody but the Owner may loosen the Agent: whatever
                    // the model was told, this is always asked.
                    Some(level) => Review::Ask(Approval {
                        summary: "Loosen the Agent's permissions".into(),
                        detail: format!(
                            "From {} to {}.\n\n{}",
                            self.level().name(),
                            level.name(),
                            level.meaning()
                        ),
                        critical: true,
                        ..Approval::default()
                    }),
                }
            }
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
                        ..Approval::default()
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
                    ..Approval::default()
                }),
            },
            _ => return None,
        })
    }

    /// Carries out an Action on Routines the gate let through.
    fn carry_out_routine(&mut self, request: &ActionRequest) -> String {
        let now = (self.clock)();
        let done = match request.name.as_str() {
            "set_permission_level" => {
                let asked = request.input["level"].as_str().unwrap_or_default();
                let Some(level) = Level::named(asked) else {
                    return format!(
                        "There is no level called \"{asked}\". The levels are: careful, balanced, free."
                    );
                };
                self.store.set_setting("permission_level", Some(level.name())).map(|()| {
                    format!("The permission level is now {}: {}", level.name(), level.meaning())
                })
            }
            "list_always_allowed" => self.always_allowed().map(|allowed| {
                if allowed.is_empty() {
                    return "Nothing is always allowed.".to_owned();
                }
                allowed.iter().map(|what| format!("- {what}")).collect::<Vec<_>>().join("\n")
            }),
            "revoke_always_allowed" => {
                let what = request.input["what"].as_str().unwrap_or_default().trim().to_owned();
                self.always_allowed().and_then(|allowed| {
                    let (taken_back, kept): (Vec<String>, Vec<String>) = allowed
                        .into_iter()
                        .partition(|entry| !what.is_empty() && entry.contains(&what));
                    let stored = serde_json::to_string(&kept).unwrap_or_default();
                    self.store.set_setting("always_allowed", Some(&stored))?;
                    Ok(if taken_back.is_empty() {
                        format!("Nothing always allowed matches \"{what}\".")
                    } else {
                        format!("No longer always allowed: {}.", taken_back.join("; "))
                    })
                })
            }
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

/// How much the Agent does without asking, from least to most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Careful,
    Balanced,
    Free,
}

impl Level {
    fn named(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "careful" => Some(Self::Careful),
            "balanced" => Some(Self::Balanced),
            "free" => Some(Self::Free),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Careful => "careful",
            Self::Balanced => "balanced",
            Self::Free => "free",
        }
    }

    fn meaning(self) -> &'static str {
        match self {
            Self::Careful => "the Agent asks before anything that changes something.",
            Self::Balanced => {
                "the Agent acts without asking inside its own folders, and asks for everything else \
                 that changes something."
            }
            Self::Free => {
                "the Agent acts without asking, except for critical actions (administrator \
                 rights, wiping, secrets, spending money), which it always asks about."
            }
        }
    }
}

/// What an "always allow" answered for `action` covers: this very command,
/// this file, this name. Nothing broader.
fn covered_by_always(action: &ActionRequest) -> String {
    let what = ["command", "path", "url", "name"]
        .iter()
        .find_map(|field| action.input[*field].as_str())
        .unwrap_or_default();
    format!("{}: {}", action.name, what.trim())
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
            // Providers explain rate limits at length; the start says it all.
            let reason = clip(reason, 160);
            format!("My AI provider is too busy to answer ({reason}). Try again later.")
        }
        ProviderError::Failed(reason) => {
            format!("My AI provider failed to answer ({reason}). Try again in a moment.")
        }
    }
}
