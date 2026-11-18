//! Orbite: a personal AI agent that lives on a machine its Owner controls.
//! Vocabulary follows `GLOSSARY.md`.

mod agent;
mod channel;
mod openai_compatible;
mod provider;
mod terminal;

pub use agent::Agent;
pub use channel::Channel;
pub use openai_compatible::OpenAiCompatible;
pub use provider::{Message, Provider, ProviderError, Speaker};
pub use terminal::Terminal;
