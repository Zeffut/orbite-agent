//! Orbite: a personal AI agent that lives on a machine its Owner controls.
//! Vocabulary follows `GLOSSARY.md`.

mod actions;
mod adopting;
mod agent;
mod anthropic;
mod attachments;
mod both;
mod catalogue;
mod channel;
mod chatgpt;
mod computer;
mod config;
mod connectors;
mod desk;
mod door;
mod extensions;
mod front;
mod hearing;
mod held;
pub use held::{keep_what_the_server_gave, server_at, server_token};
mod interface;
mod language;
mod look;
mod mcp;
mod memory;
mod moving;
mod openai_compatible;
mod png;
mod provider;
mod providers;
mod reach;
mod routed;
mod routines;
mod secrets;
mod served;
mod server;
mod settings;
mod setup;
mod skills;
mod slug;
mod steps;
mod store;
mod subagents;
mod talk;
mod telegram;
mod terminal;
mod transcriber;
mod tryout;
mod updater;
mod web;
mod welcome;

pub use adopting::{adopt, as_the_agent, as_the_server, keep as keep_agents};
pub use agent::Agent;
pub use anthropic::Anthropic;
pub use attachments::attached;
pub use both::Both;
pub use catalogue::{
    Connected, KnownProvider, Speaks, connect, connect_in, known_providers, transcription_model,
};
pub use channel::{Answer, Approval, Channel};
pub use chatgpt::ChatGpt;
pub use computer::{ByHand, Computer, Hand, Screen};
pub use config::{Config, ProviderChoice, Transcription};
pub use connectors::{CONNECTORS, Connector};
pub use desk::{
    NotOffered, SOCKET as DESK_SOCKET, lending as lending_this_computer,
    offer as offer_this_computer,
};
pub use door::{Door, is_of_the_internet, public_address};
pub use front::{Choice, Front, Left, Lines, Mood, Picked};
pub use hearing::{Advice, MODELS, Machine, Model, chosen as chosen_transcriber, transcription};
pub use interface::{
    Device, Interface, devices as interface_devices, is_private, pairing_code, turn_away,
};
pub use language::Language;
pub use look::{COLOURS, Look, SHAPES};
pub use mcp::{
    add_server as add_connector, listed as connectors, remove_server as remove_connector,
    turned_off as connectors_turned_off,
};
pub use moving::{export, restore};
pub use openai_compatible::OpenAiCompatible;
pub use provider::{
    ActionRequest, ActionSpec, Message, Provider, ProviderError, Reply, Request, Usage,
};
pub use providers::{change_providers, change_providers_in, providers_shown};
pub use reach::{close_on_tailscale, open_on_tailscale, tailscale};
pub use routed::Routed;
pub use secrets::{Secrets, SecretsError};
pub use served::{Asked, Served};
pub use server::{Docker, Engine, Hears, Held, Server, Thinks, held_in};
pub use settings::{SETTINGS, Settings, TAKES_A_RESTART};
pub use setup::{PROVIDER_KEY, Setup, SetupError, Step as SetupStep, TELEGRAM_TOKEN};
pub use steps::Step;
pub use store::{Entry, Store, StoreError};
pub use talk::talk;
pub use telegram::Telegram;
pub use terminal::Terminal;
pub use transcriber::Transcriber;
pub use tryout::{Tools, Verdict, advice, keep_rating, rating, try_out};
pub use updater::{
    RELEASE_KEY, Service, Systemd, UpdateError, Updater, run_by_sudo_for_another, system_program,
};
pub use welcome::{Welcome, code_picture, square_code_text};
