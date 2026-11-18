//! Orbite: a personal AI agent that lives on a machine its Owner controls.
//! Vocabulary follows `GLOSSARY.md`.

mod agent;
mod channel;
mod openai_compatible;
mod provider;
mod store;
mod telegram;
mod terminal;

pub use agent::Agent;
pub use channel::Channel;
pub use openai_compatible::OpenAiCompatible;
pub use provider::{ActionRequest, ActionSpec, Message, Provider, ProviderError, Reply, Request};
pub use store::{Store, StoreError};
pub use telegram::Telegram;
pub use terminal::Terminal;
