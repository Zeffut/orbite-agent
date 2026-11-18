//! Orbite: a personal AI agent that lives on a machine its Owner controls.
//! Vocabulary follows `GLOSSARY.md`.

mod agent;
mod channel;
mod openai_compatible;
mod provider;
mod store;
mod terminal;

pub use agent::Agent;
pub use channel::Channel;
pub use openai_compatible::OpenAiCompatible;
pub use provider::{Message, Provider, ProviderError, Speaker};
pub use store::{Store, StoreError};
pub use terminal::Terminal;
