//! The seam most tests use: the whole Agent, driven as its Owner through an
//! in-memory Channel, thinking with a Provider whose answers are scripted.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orbite::{
    ActionRequest, Answer, Approval, Channel, Message, Provider, ProviderError, Reply, Request,
    Step,
};
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
    instructed: Arc<Mutex<Vec<String>>>,
    offered: Arc<Mutex<Vec<Vec<String>>>>,
    shown_images: Arc<Mutex<Vec<Images>>>,
    otherwise: Arc<Mutex<Option<Responder>>>,
    held: Arc<Mutex<Option<Hold>>>,
}

/// The images shown on one call, each with the place of its message.
type Images = Vec<(usize, std::path::PathBuf)>;

/// Calls whose instructions contain the text wait for the signal.
type Hold = (String, tokio::sync::watch::Receiver<bool>);

type Responder = Box<dyn Fn(&str, &[Message]) -> Result<Reply, ProviderError> + Send>;

/// A Reply that only says `text`.
pub fn saying(text: &str) -> Reply {
    Reply { text: text.to_string(), actions: Vec::new(), kept: None }
}

/// A Reply that asks for one Action.
pub fn acting(name: &str, input: Value) -> Reply {
    Reply {
        text: String::new(),
        actions: vec![ActionRequest { id: format!("{name}-0"), name: name.to_string(), input }],
        kept: None,
    }
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
        let outcome = outcome.map(|text| Reply { text, actions: Vec::new(), kept: None });
        self.script.lock().unwrap().push_back(Outcome::Now(outcome));
        self
    }

    /// The model asks for these Actions, each given as (name, input).
    pub fn then_acts<const N: usize>(&self, actions: [(&str, Value); N]) -> &Self {
        self.then_says_and_acts("", actions)
    }

    pub fn then_says_and_acts<const N: usize>(
        &self,
        text: &str,
        actions: [(&str, Value); N],
    ) -> &Self {
        let actions = actions
            .into_iter()
            .enumerate()
            .map(|(index, (name, input))| ActionRequest {
                id: format!("{name}-{index}"),
                name: name.to_string(),
                input,
            })
            .collect();
        let reply = Reply { text: text.to_string(), actions, kept: None };
        self.script.lock().unwrap().push_back(Outcome::Now(Ok(reply)));
        self
    }

    /// Once the script is used up, answers are computed from the
    /// instructions and Conversation of each call.
    pub fn otherwise(
        &self,
        respond: impl Fn(&str, &[Message]) -> Result<String, ProviderError> + Send + 'static,
    ) -> &Self {
        self.replying_otherwise(move |instructions, conversation| {
            respond(instructions, conversation).map(|text| saying(&text))
        })
    }

    /// As [`ScriptedProvider::otherwise`], for answers that may ask for Actions.
    pub fn replying_otherwise(
        &self,
        respond: impl Fn(&str, &[Message]) -> Result<Reply, ProviderError> + Send + 'static,
    ) -> &Self {
        *self.otherwise.lock().unwrap() = Some(Box::new(respond));
        self
    }

    /// Calls whose instructions contain `marker` wait until the returned
    /// handle is told `true`.
    pub fn holding(&self, marker: &str) -> tokio::sync::watch::Sender<bool> {
        let (release, released) = tokio::sync::watch::channel(false);
        *self.held.lock().unwrap() = Some((marker.to_string(), released));
        release
    }

    /// The names of the Actions on offer, call after call, with the instructions.
    pub fn every_offer(&self) -> Vec<(String, Vec<String>)> {
        let instructed = self.instructed.lock().unwrap().clone();
        instructed.into_iter().zip(self.offered.lock().unwrap().clone()).collect()
    }

    /// Everything the Provider was asked, call after call: (instructions, Conversation).
    pub fn every_call(&self) -> Vec<(String, Vec<Message>)> {
        let instructed = self.instructed.lock().unwrap().clone();
        instructed.into_iter().zip(self.asked.lock().unwrap().clone()).collect()
    }

    /// The next call never returns, as when Orbite is stopped mid-answer.
    pub fn then_never_answers(&self) -> &Self {
        self.script.lock().unwrap().push_back(Outcome::Never);
        self
    }

    pub fn times_asked(&self) -> usize {
        self.asked.lock().unwrap().len()
    }

    /// The instructions the Provider was given on its latest call.
    pub fn last_instructions(&self) -> String {
        self.instructed.lock().unwrap().last().cloned().expect("the Provider was never asked")
    }

    /// The images shown to the model on its latest call, each with the place
    /// in the Conversation of the message it came with.
    pub fn last_images(&self) -> Vec<(usize, std::path::PathBuf)> {
        self.shown_images.lock().unwrap().last().cloned().expect("the Provider was never asked")
    }

    /// The names of the Actions on offer on the Provider's latest call.
    pub fn last_offered(&self) -> Vec<String> {
        self.offered.lock().unwrap().last().cloned().expect("the Provider was never asked")
    }

    /// The Conversation as the Provider saw it on its latest call.
    pub fn last_asked(&self) -> Vec<Message> {
        self.asked.lock().unwrap().last().cloned().expect("the Provider was never asked")
    }
}

impl Provider for ScriptedProvider {
    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        self.asked.lock().unwrap().push(request.conversation.to_vec());
        self.instructed.lock().unwrap().push(request.instructions.to_string());
        self.shown_images.lock().unwrap().push(request.images.to_vec());
        self.offered
            .lock()
            .unwrap()
            .push(request.actions.iter().map(|action| action.name.clone()).collect());
        let held = self.held.lock().unwrap().clone();
        if let Some((marker, mut released)) = held
            && request.instructions.contains(&marker)
        {
            let _ = released.wait_for(|released| *released).await;
        }
        let scripted = self.script.lock().unwrap().pop_front();
        let Some(outcome) = scripted else {
            let otherwise = self.otherwise.lock().unwrap();
            let respond = otherwise.as_ref().expect("the script ran out of answers");
            return respond(request.instructions, request.conversation);
        };
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
    asks: mpsc::UnboundedSender<Approval>,
    answers: mpsc::UnboundedReceiver<Answer>,
    shown: Arc<Mutex<Vec<String>>>,
}

/// The Owner's end: what a person would type, read and tap.
pub struct Owner {
    to_agent: mpsc::UnboundedSender<String>,
    from_agent: mpsc::UnboundedReceiver<String>,
    asked: mpsc::UnboundedReceiver<Approval>,
    answers: mpsc::UnboundedSender<Answer>,
    shown: Arc<Mutex<Vec<String>>>,
}

pub fn memory_channel() -> (MemoryChannel, Owner) {
    let (to_agent, from_owner) = mpsc::unbounded_channel();
    let (to_owner, from_agent) = mpsc::unbounded_channel();
    let (asks, asked) = mpsc::unbounded_channel();
    let (answers_in, answers_out) = mpsc::unbounded_channel();
    let shown = Arc::new(Mutex::new(Vec::new()));
    (
        MemoryChannel { from_owner, to_owner, asks, answers: answers_out, shown: shown.clone() },
        Owner { to_agent, from_agent, asked, answers: answers_in, shown },
    )
}

impl Channel for MemoryChannel {
    async fn receive(&mut self) -> Option<String> {
        self.from_owner.recv().await
    }

    async fn send(&mut self, text: &str) -> std::io::Result<()> {
        self.to_owner.send(text.to_string()).map_err(std::io::Error::other)
    }

    async fn interruption(&mut self) -> Option<String> {
        self.from_owner.recv().await
    }

    async fn send_file(&mut self, file: &std::path::Path, caption: &str) -> std::io::Result<()> {
        let content = std::fs::read_to_string(file)?;
        self.to_owner
            .send(format!(
                "(file {}: {content}) {caption}",
                file.file_name().unwrap().to_string_lossy()
            ))
            .map_err(std::io::Error::other)
    }

    async fn echo(&mut self, text: &str) {
        let _ = self.to_owner.send(format!("(you, elsewhere) {text}"));
    }

    async fn settle(&mut self, _approval: &Approval, answer: &Answer) {
        let _ = self.to_owner.send(format!("(answered elsewhere) {answer:?}"));
    }

    async fn doing(&mut self, step: &Step) {
        self.shown.lock().unwrap().push(format!("{} {}", step.icon, step.text));
    }

    async fn idle(&mut self) {
        self.shown.lock().unwrap().push("(idle)".to_string());
    }

    async fn ask(&mut self, approval: &Approval) -> std::io::Result<Answer> {
        self.asks.send(approval.clone()).map_err(std::io::Error::other)?;
        tokio::select! {
            answer = self.answers.recv() => {
                answer.ok_or_else(|| std::io::Error::other("the Owner is gone"))
            }
            Some(text) = self.from_owner.recv() => Ok(Answer::Wrote(text)),
        }
    }
}

impl Owner {
    pub fn says(&self, text: &str) {
        self.to_agent.send(text.to_string()).expect("the Agent is gone");
    }

    /// True when the Agent says nothing for a moment.
    pub async fn hears_nothing(&mut self) -> bool {
        tokio::time::timeout(Duration::from_millis(100), self.from_agent.recv()).await.is_err()
    }

    pub async fn hears(&mut self) -> String {
        tokio::time::timeout(Duration::from_secs(5), self.from_agent.recv())
            .await
            .expect("the Agent stayed silent")
            .expect("the Agent is gone")
    }

    /// The Approval the Agent is waiting on.
    pub async fn is_asked(&mut self) -> Approval {
        tokio::time::timeout(Duration::from_secs(5), self.asked.recv())
            .await
            .expect("the Agent asked for nothing")
            .expect("the Agent is gone")
    }

    /// What the Owner was shown of the Agent's work, in order; "(idle)"
    /// marks each time the Agent had nothing under way any more.
    pub fn saw(&self) -> Vec<String> {
        self.shown.lock().unwrap().clone()
    }

    pub fn was_asked_nothing(&mut self) -> bool {
        self.asked.try_recv().is_err()
    }

    pub fn allows(&self) {
        self.answers.send(Answer::Allowed).expect("the Agent is gone");
    }

    pub fn allows_always(&self) {
        self.answers.send(Answer::AlwaysAllowed).expect("the Agent is gone");
    }

    pub fn denies(&self) {
        self.answers.send(Answer::Denied).expect("the Agent is gone");
    }
}
