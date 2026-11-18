use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One entry of the Conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// Something the Owner said.
    Owner(String),
    /// Something the Agent said, and the Actions it asked for while saying it.
    Agent { text: String, actions: Vec<ActionRequest> },
    /// What came of the Action whose id is `action`.
    Outcome { action: String, text: String },
    /// Something the Agent is told that its Owner did not say: a Sub-agent's
    /// report, for instance.
    Notice(String),
}

impl Message {
    pub fn from_owner(text: impl Into<String>) -> Self {
        Self::Owner(text.into())
    }

    pub fn from_agent(text: impl Into<String>) -> Self {
        Self::Agent { text: text.into(), actions: Vec::new() }
    }
}

/// An Action the model asks the Agent to carry out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRequest {
    /// Chosen by the Provider; ties the request to its Outcome.
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// An Action the model may ask for.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionSpec {
    pub name: String,
    /// Tells the model what the Action does and when to use it.
    pub description: String,
    /// JSON Schema of the Action's input.
    pub input: Value,
}

/// What a Provider is asked on each turn.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// Who the Agent is and how it should behave.
    pub instructions: &'a str,
    pub conversation: &'a [Message],
    pub actions: &'a [ActionSpec],
}

/// What the model says next, and the Actions it asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub text: String,
    pub actions: Vec<ActionRequest>,
}

/// Something that runs AI models for the Agent.
pub trait Provider: Send + Sync {
    fn reply(
        &self,
        request: Request<'_>,
    ) -> impl Future<Output = Result<Reply, ProviderError>> + Send;
}

/// Why a Provider gave no answer. The text is the Provider's own explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// The Provider refused the API Key or Subscription.
    Rejected(String),
    /// The Provider could not be reached at all.
    Unreachable(String),
    /// The Provider cannot answer right now but may shortly: a rate limit or
    /// an overload. `retry_after` is the wait it asked for, when it said.
    Busy { reason: String, retry_after: Option<Duration> },
    /// The Provider was reached but failed to answer.
    Failed(String),
}
