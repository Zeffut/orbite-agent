use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use crate::actions::{Host, Review};
use crate::memory::Memory;
use crate::{Channel, Message, Provider, ProviderError, Request, Store, StoreError};

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

/// Channels show a working signal for a few seconds only; it is renewed at
/// this pace while the Agent works.
const WORKING_SIGNAL_EVERY: Duration = Duration::from_secs(4);

/// How many times in a row the model may act before the Agent stops and
/// hands back to its Owner.
const STEPS_BEFORE_STOPPING: usize = 50;

/// The single persistent AI worker of an installation.
pub struct Agent<P> {
    provider: P,
    store: Store,
    host: Host,
}

impl<P: Provider> Agent<P> {
    pub fn new(provider: P, store: Store) -> Self {
        let host = Host {
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            command_timeout: Duration::from_secs(120),
            memory: None,
        };
        Self { provider, store, host }
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

    /// Who the Agent is and what it remembers, as told to the model.
    fn instructions(&self) -> String {
        let Some(memory) = &self.host.memory else { return INSTRUCTIONS.to_owned() };
        let mut instructions = format!("{INSTRUCTIONS}\n\n{MEMORY_INSTRUCTIONS}\n");
        let entries = memory.entries();
        if entries.is_empty() {
            instructions.push_str("\n(nothing yet)");
        }
        for (name, text) in entries {
            instructions.push_str(&format!("\n- {name}: {text}"));
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
        let actions = self.host.specs();
        for _ in 0..STEPS_BEFORE_STOPPING {
            let conversation = self.store.conversation()?;
            let instructions = self.instructions();
            let request = Request {
                instructions: &instructions,
                conversation: &conversation,
                actions: &actions,
            };
            let reply = match showing_work(channel, self.provider.reply(request)).await {
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
        ProviderError::Failed(reason) => {
            format!("My AI provider failed to answer ({reason}). Try again in a moment.")
        }
    }
}
