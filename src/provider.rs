use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One entry of the Conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// Something the Owner said.
    Owner(String),
    /// Something the Agent said, and the Actions it asked for while saying it.
    Agent {
        text: String,
        actions: Vec<ActionRequest>,
        /// What the Provider that wrote this needs back exactly as it was,
        /// such as reasoning only it can read, under that Provider's name.
        /// Nothing else in Orbite looks inside.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kept: Option<Value>,
    },
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
        Self::Agent { text: text.into(), actions: Vec::new(), kept: None }
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
    /// Images to show the model, each with the place in `conversation` of the
    /// message it came with.
    pub images: &'a [(usize, PathBuf)],
    /// True for work a lighter, cheaper model does as well: a summary, a
    /// Wake-up. A Provider with only one model ignores it.
    pub light: bool,
}

/// What the model says next, and the Actions it asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub text: String,
    pub actions: Vec<ActionRequest>,
    /// What the Provider needs back exactly as it was on later turns; see
    /// [`Message::Agent`].
    pub kept: Option<Value>,
    /// Who answered and what it cost, when the Provider says.
    pub usage: Option<Usage>,
}

/// What one answer cost, as its Provider counts it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    /// The Provider that answered, by the name it was connected under.
    pub by: String,
    pub model: String,
    /// Tokens read, including those read from the Provider's cache.
    pub input: u64,
    /// Of `input`, the tokens read from the cache, which cost far less.
    pub cached: u64,
    /// Tokens written.
    pub output: u64,
}

/// Something that runs AI models for the Agent.
pub trait Provider: Send + Sync {
    fn reply(
        &self,
        request: Request<'_>,
    ) -> impl Future<Output = Result<Reply, ProviderError>> + Send;

    /// The name of the Provider that normally answers, when several stand
    /// behind this one, so that the Owner can be told when another does.
    fn usual(&self) -> Option<&str> {
        None
    }
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
    /// The model itself declined the request, for the reason given.
    Declined(String),
    /// The Provider was reached but failed to answer.
    Failed(String),
}
