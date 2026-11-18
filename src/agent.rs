use crate::{Channel, Message, Provider, ProviderError};

/// The single persistent AI worker of an installation.
pub struct Agent<P> {
    provider: P,
    conversation: Vec<Message>,
}

impl<P: Provider> Agent<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            conversation: Vec::new(),
        }
    }

    /// Holds the Conversation on `channel` until the Channel closes.
    pub async fn converse(&mut self, channel: &mut impl Channel) {
        while let Some(text) = channel.receive().await {
            let answer = self.answer(text).await;
            if channel.send(&answer).await.is_err() {
                break;
            }
        }
    }

    async fn answer(&mut self, text: String) -> String {
        self.conversation.push(Message::from_owner(text));
        match self.provider.reply(&self.conversation).await {
            Ok(answer) => {
                self.conversation.push(Message::from_agent(answer.clone()));
                answer
            }
            Err(error) => {
                // The Owner will say it again: an unanswered message must not
                // stay in the Conversation and be answered twice.
                self.conversation.pop();
                explain(&error)
            }
        }
    }
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
