//! Orbite: a personal AI agent that lives on a machine its Owner controls.
//! Vocabulary follows `GLOSSARY.md`.

mod actions;
mod agent;
mod anthropic;
mod attachments;
mod both;
mod catalogue;
mod channel;
mod config;
mod extensions;
mod language;
mod mcp;
mod memory;
mod moving;
mod openai_compatible;
mod provider;
mod routed;
mod routines;
mod secrets;
mod settings;
mod setup;
mod skills;
mod slug;
mod steps;
mod store;
mod subagents;
mod telegram;
mod terminal;
mod tryout;
mod updater;
mod web;

pub use agent::Agent;
pub use anthropic::Anthropic;
pub use attachments::attached;
pub use both::Both;
pub use catalogue::{Connected, KnownProvider, Speaks, connect, known_providers};
pub use channel::{Answer, Approval, Channel};
pub use config::{Config, ProviderChoice};
pub use language::Language;
pub use moving::{export, restore};
pub use openai_compatible::OpenAiCompatible;
pub use provider::{
    ActionRequest, ActionSpec, Message, Provider, ProviderError, Reply, Request, Usage,
};
pub use routed::Routed;
pub use secrets::{Secrets, SecretsError};
pub use settings::{SETTINGS, Settings, TAKES_A_RESTART};
pub use setup::{PROVIDER_KEY, Setup, SetupError, Step as SetupStep, TELEGRAM_TOKEN};
pub use steps::Step;
pub use store::{Entry, Store, StoreError};
pub use telegram::Telegram;
pub use terminal::Terminal;
pub use tryout::{Tools, Verdict, try_out};
pub use updater::{RELEASE_KEY, Service, Systemd, UpdateError, Updater};
