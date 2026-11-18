use std::time::Duration;

use crate::{Channel, Message, Provider, ProviderError, Request, Store, StoreError};

/// Who the Agent is, told to the model on every turn.
const INSTRUCTIONS: &str = "\
You are a personal AI agent running on a machine that belongs to your Owner, \
the only person you talk to and work for. Answer in the language your Owner \
writes in. Be direct and brief.";

/// Channels show a working signal for a few seconds only; it is renewed at
/// this pace while the Agent thinks.
const WORKING_SIGNAL_EVERY: Duration = Duration::from_secs(4);

/// The single persistent AI worker of an installation.
pub struct Agent<P> {
    provider: P,
    store: Store,
}

impl<P: Provider> Agent<P> {
    pub fn new(provider: P, store: Store) -> Self {
        Self { provider, store }
    }

    /// Holds the Conversation on `channel` until the Channel closes.
    pub async fn converse(&mut self, channel: &mut impl Channel) {
        // A message the Owner sent just before Orbite stopped is still owed
        // an answer.
        if self.owes_an_answer() && !self.answer_on(channel).await {
            return;
        }
        while let Some(text) = channel.receive().await {
            if let Err(error) = self.store.append(&Message::from_owner(text)) {
                if channel.send(&explain_storage(&error)).await.is_err() {
                    return;
                }
                continue;
            }
            if !self.answer_on(channel).await {
                return;
            }
        }
    }

    fn owes_an_answer(&self) -> bool {
        matches!(
            self.store.conversation().as_deref(),
            Ok([.., Message::Owner(_)])
        )
    }

    /// Answers the Owner's latest message; false once the Channel is closed.
    async fn answer_on(&mut self, channel: &mut impl Channel) -> bool {
        let answer = self
            .answer(channel)
            .await
            .unwrap_or_else(|error| explain_storage(&error));
        channel.send(&answer).await.is_ok()
    }

    async fn answer(&mut self, channel: &mut impl Channel) -> Result<String, StoreError> {
        let conversation = self.store.conversation()?;
        let reply = {
            let thinking = self.provider.reply(Request {
                instructions: INSTRUCTIONS,
                conversation: &conversation,
                actions: &[],
            });
            tokio::pin!(thinking);
            loop {
                channel.working().await;
                tokio::select! {
                    reply = &mut thinking => break reply,
                    () = tokio::time::sleep(WORKING_SIGNAL_EVERY) => {}
                }
            }
        };
        match reply {
            Ok(reply) => {
                self.store
                    .append(&Message::from_agent(reply.text.clone()))?;
                Ok(reply.text)
            }
            Err(error) => {
                // The Owner will say it again: a message the Provider failed
                // on must not stay in the Conversation and be answered twice.
                self.store.forget_latest()?;
                Ok(explain(&error))
            }
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
