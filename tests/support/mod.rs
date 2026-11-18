//! The seam most tests use: the whole Agent, driven as its Owner through an
//! in-memory Channel, thinking with a Provider whose answers are scripted.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orbite::{ActionRequest, Channel, Message, Provider, ProviderError, Reply, Request};
use serde_json::Value;
use tokio::sync::mpsc;

enum Outcome {
    Now(Result<Reply, ProviderError>),
    Never,
}

/// A Provider that gives the next scripted answer and records what it was asked.
#[derive(Clone, Default)]
pub struct ScriptedProvider {
    script: Arc<Mutex<VecDeque<Outcome>>>,
    asked: Arc<Mutex<Vec<Vec<Message>>>>,
}

impl ScriptedProvider {
    pub fn answering<const N: usize>(answers: [&str; N]) -> Self {
        let provider = Self::default();
        for answer in answers {
            provider.then(Ok(answer.to_string()));
        }
        provider
    }

    pub fn then(&self, outcome: Result<String, ProviderError>) -> &Self {
        let outcome = outcome.map(|text| Reply {
            text,
            actions: Vec::new(),
        });
        self.script.lock().unwrap().push_back(Outcome::Now(outcome));
        self
    }

    /// The model asks for these Actions, each given as (name, input).
    pub fn then_acts<const N: usize>(&self, actions: [(&str, Value); N]) -> &Self {
        let actions = actions
            .into_iter()
            .enumerate()
            .map(|(index, (name, input))| ActionRequest {
                id: format!("{name}-{index}"),
                name: name.to_string(),
                input,
            })
            .collect();
        let reply = Reply {
            text: String::new(),
            actions,
        };
        self.script
            .lock()
            .unwrap()
            .push_back(Outcome::Now(Ok(reply)));
        self
    }

    /// The next call never returns, as when Orbite is stopped mid-answer.
    pub fn then_never_answers(&self) -> &Self {
        self.script.lock().unwrap().push_back(Outcome::Never);
        self
    }

    pub fn times_asked(&self) -> usize {
        self.asked.lock().unwrap().len()
    }

    /// The Conversation as the Provider saw it on its latest call.
    pub fn last_asked(&self) -> Vec<Message> {
        self.asked
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("the Provider was never asked")
    }
}

impl Provider for ScriptedProvider {
    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        self.asked
            .lock()
            .unwrap()
            .push(request.conversation.to_vec());
        let outcome = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .expect("the script ran out of answers");
        match outcome {
            Outcome::Now(outcome) => outcome,
            Outcome::Never => std::future::pending().await,
        }
    }
}

/// The Agent's end of an in-memory Channel.
pub struct MemoryChannel {
    from_owner: mpsc::UnboundedReceiver<String>,
    to_owner: mpsc::UnboundedSender<String>,
}

/// The Owner's end: what a person would type and read.
pub struct Owner {
    to_agent: mpsc::UnboundedSender<String>,
    from_agent: mpsc::UnboundedReceiver<String>,
}

pub fn memory_channel() -> (MemoryChannel, Owner) {
    let (to_agent, from_owner) = mpsc::unbounded_channel();
    let (to_owner, from_agent) = mpsc::unbounded_channel();
    (
        MemoryChannel {
            from_owner,
            to_owner,
        },
        Owner {
            to_agent,
            from_agent,
        },
    )
}

impl Channel for MemoryChannel {
    async fn receive(&mut self) -> Option<String> {
        self.from_owner.recv().await
    }

    async fn send(&mut self, text: &str) -> std::io::Result<()> {
        self.to_owner
            .send(text.to_string())
            .map_err(std::io::Error::other)
    }
}

impl Owner {
    pub fn says(&self, text: &str) {
        self.to_agent
            .send(text.to_string())
            .expect("the Agent is gone");
    }

    /// True when the Agent says nothing for a moment.
    pub async fn hears_nothing(&mut self) -> bool {
        tokio::time::timeout(Duration::from_millis(100), self.from_agent.recv())
            .await
            .is_err()
    }

    pub async fn hears(&mut self) -> String {
        tokio::time::timeout(Duration::from_secs(2), self.from_agent.recv())
            .await
            .expect("the Agent stayed silent")
            .expect("the Agent is gone")
    }
}
