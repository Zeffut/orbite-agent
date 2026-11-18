use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use crate::actions::{Host, Review};
use crate::memory::Memory;
use crate::skills::Skills;
use crate::{
    ActionRequest, ActionSpec, Channel, Message, Provider, ProviderError, Request, Store,
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

/// How many times a busy Provider is asked before the Owner is told.
const TRIES_WHILE_BUSY: usize = 5;

/// The longest a busy Provider is waited for between two tries.
const LONGEST_WAIT: Duration = Duration::from_secs(30);

/// How many times in a row the model may act before the Agent stops and
/// hands back to its Owner.
const STEPS_BEFORE_STOPPING: usize = 50;

/// The single persistent AI worker of an installation.
pub struct Agent<P> {
    provider: P,
    store: Store,
    host: Host,
    /// The most the model is sent on a turn, in tokens (estimated).
    context_budget: usize,
}

impl<P: Provider> Agent<P> {
    pub fn new(provider: P, store: Store) -> Self {
        let host = Host {
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            command_timeout: Duration::from_secs(120),
            memory: None,
            skills: None,
        };
        Self { provider, store, host, context_budget: 60_000 }
    }

    /// The folder where the Agent's commands run and its relative paths start.
    pub fn working_in(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.host.workspace = workspace.into();
        self
    }

    /// The folder where the Agent keeps its Memory, one file per entry.
    pub fn remembering_in(mut self, folder: impl Into<PathBuf>) -> Self {
        self.host.memory = Some(Memory::kept_in(folder.into()));
        self
    }

    /// The folder where the Agent keeps its Skills, one folder per Skill.
    pub fn learning_in(mut self, folder: impl Into<PathBuf>) -> Self {
        self.host.skills = Some(Skills::kept_in(folder.into()));
        self
    }

    /// The most the model is sent on a turn. Orbite counts four characters
    /// to a token, which is close enough for every Provider. Beyond it, the
    /// oldest part of the Conversation is replaced by a summary.
    pub fn keeping_context_under(mut self, tokens: usize) -> Self {
        self.context_budget = tokens;
        self
    }

    /// A command still running after `limit` is stopped.
    pub fn stopping_commands_after(mut self, limit: Duration) -> Self {
        self.host.command_timeout = limit;
        self
    }

    /// Holds the Conversation on `channel` until the Channel closes.
    pub async fn converse(&mut self, channel: &mut impl Channel) {
        // Whatever was left unfinished when Orbite last stopped comes first.
        if self.unfinished().unwrap_or(false) && !self.respond_on(channel).await {
            return;
        }
        while let Some(text) = channel.receive().await {
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
            || matches!(conversation.last(), Some(Message::Owner(_) | Message::Outcome { .. })))
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
        for _ in 0..STEPS_BEFORE_STOPPING {
            self.fit(channel, &actions).await?;
            let mut conversation = self.store.conversation()?;
            // Every protocol expects the Owner to speak first.
            let opened = !matches!(conversation.first(), Some(Message::Owner(_)) | None);
            if opened {
                conversation.insert(0, Message::from_owner(SUMMARISED_NOTE));
            }
            let instructions = self.instructions();
            let request = Request {
                instructions: &instructions,
                conversation: &conversation,
                actions: &actions,
            };
            let reply = match showing_work(channel, patiently(&self.provider, request)).await {
                Ok(reply) => reply,
                Err(error) => {
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
            for action in &reply.actions {
                if action.name == "search_conversation" {
                    let text = self.search(action);
                    self.store.append(&Message::Outcome { action: action.id.clone(), text })?;
                    continue;
                }
                let text = match self.host.review(action) {
                    Review::Refuse(problem) => problem,
                    Review::Free => showing_work(channel, self.host.carry_out(action)).await,
                    Review::Ask(approval) => match channel.ask(&approval).await {
                        Ok(true) => showing_work(channel, self.host.carry_out(action)).await,
                        Ok(false) => "The Owner denied this action. It was not carried out.".into(),
                        Err(_) => return Ok(false),
                    },
                };
                self.store.append(&Message::Outcome { action: action.id.clone(), text })?;
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

impl<P: Provider> Agent<P> {
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
        let reply = showing_work(channel, patiently(&self.provider, request)).await.ok()?;
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
                    Message::Outcome { text, .. } => format!("{date} {}", clip(text, 600)),
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
fn clip(text: &str, limit: usize) -> &str {
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
async fn patiently(
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
fn explain(error: &ProviderError) -> String {
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
