use std::future::Future;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    Owner,
    Agent,
}

/// One thing said in the Conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub speaker: Speaker,
    pub text: String,
}

impl Message {
    pub fn from_owner(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::Owner,
            text: text.into(),
        }
    }

    pub fn from_agent(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::Agent,
            text: text.into(),
        }
    }
}

/// Something that runs AI models for the Agent.
pub trait Provider: Send + Sync {
    /// What the model says next, given the Conversation so far.
    fn reply(
        &self,
        conversation: &[Message],
    ) -> impl Future<Output = Result<String, ProviderError>> + Send;
}

/// Why a Provider gave no answer. The text is the Provider's own explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// The Provider refused the API Key or Subscription.
    Rejected(String),
    /// The Provider could not be reached at all.
    Unreachable(String),
    /// The Provider was reached but failed to answer.
    Failed(String),
}
