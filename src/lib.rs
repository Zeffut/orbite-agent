//! Orbite: a personal AI agent that lives on a machine its Owner controls.
//! Vocabulary follows `GLOSSARY.md`.

mod actions;
mod agent;
mod channel;
mod memory;
mod openai_compatible;
mod provider;
mod routines;
mod skills;
mod slug;
mod store;
mod subagents;
mod telegram;
mod terminal;
mod web;

pub use agent::Agent;
pub use channel::{Answer, Approval, Channel};
pub use openai_compatible::OpenAiCompatible;
pub use provider::{ActionRequest, ActionSpec, Message, Provider, ProviderError, Reply, Request};
pub use store::{Store, StoreError};
pub use telegram::Telegram;
pub use terminal::Terminal;
