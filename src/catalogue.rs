//! The Providers known by name, and how each is reached.

use std::path::Path;

use crate::{Anthropic, ChatGpt, OpenAiCompatible, Provider, ProviderError, Reply, Request};

/// The protocol a Provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaks {
    /// The chat-completions protocol OpenAI introduced and most others took up.
    OpenAi,
    /// Anthropic's Messages API.
    Anthropic,
    /// OpenAI's Responses API, as whoever signed in with their ChatGPT plan.
    ChatGptPlan,
}

/// A Provider Orbite can be pointed at by name.
#[derive(Debug, Clone, Copy)]
pub struct KnownProvider {
    /// What the Owner types to choose it.
    pub name: &'static str,
    /// What it is called.
    pub label: &'static str,
    pub speaks: Speaks,
    /// The root of its API; empty for `custom`, whose address the Owner gives.
    pub api_root: &'static str,
    /// False for a model running on the Owner's own machine.
    pub needs_key: bool,
}

const fn openai(name: &'static str, label: &'static str, api_root: &'static str) -> KnownProvider {
    KnownProvider { name, label, speaks: Speaks::OpenAi, api_root, needs_key: true }
}

const fn local(name: &'static str, label: &'static str, api_root: &'static str) -> KnownProvider {
    KnownProvider { name, label, speaks: Speaks::OpenAi, api_root, needs_key: false }
}

const KNOWN: &[KnownProvider] = &[
    KnownProvider {
        name: "anthropic",
        label: "Anthropic (Claude)",
        speaks: Speaks::Anthropic,
        api_root: "https://api.anthropic.com",
        needs_key: true,
    },
    openai("openai", "OpenAI", "https://api.openai.com/v1"),
    KnownProvider {
        name: "chatgpt",
        label: "ChatGPT plan (Plus or Pro, no API key)",
        speaks: Speaks::ChatGptPlan,
        api_root: "https://api.openai.com/v1",
        needs_key: false,
    },
    openai("google", "Google (Gemini)", "https://generativelanguage.googleapis.com/v1beta/openai"),
    openai("xai", "xAI (Grok)", "https://api.x.ai/v1"),
    openai("mistral", "Mistral", "https://api.mistral.ai/v1"),
    openai("deepseek", "DeepSeek", "https://api.deepseek.com/v1"),
    openai("groq", "Groq", "https://api.groq.com/openai/v1"),
    openai("cerebras", "Cerebras", "https://api.cerebras.ai/v1"),
    openai("together", "Together AI", "https://api.together.xyz/v1"),
    openai("fireworks", "Fireworks AI", "https://api.fireworks.ai/inference/v1"),
    openai("openrouter", "OpenRouter (many models, one key)", "https://openrouter.ai/api/v1"),
    local("ollama", "Ollama (on this machine)", "http://localhost:11434/v1"),
    local("lmstudio", "LM Studio (on this machine)", "http://localhost:1234/v1"),
];

/// The model with which the Provider called `name` writes speech down,
/// when it offers that to whoever has an API Key.
pub fn transcription_model(name: &str) -> Option<&'static str> {
    match name {
        "openai" => Some("gpt-transcribe"),
        "groq" => Some("whisper-large-v3-turbo"),
        _ => None,
    }
}

/// The models the Provider called `name` is known to serve an Agent well
/// with, the first that answers being taken at setup, then those for light
/// work (summaries, wake-ups). Read on each Provider's own pages on
/// 2026-11-23; empty where the Owner is to choose.
pub fn usual_models(name: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match name {
        "anthropic" => (&["claude-sonnet-5-5", "claude-opus-5-5"], &["claude-haiku-5-5"]),
        "openai" => (&["gpt-6.2-sol", "gpt-6.1-terra"], &[]),
        "chatgpt" => (&["gpt-6.2-sol", "gpt-6.1-terra", "gpt-6-luna"], &[]),
        "google" => (
            &["gemini-4-hydrogen", "gemini-3.8-flash", "gemini-3.6-flash"],
            &["gemini-3.5-flash-lite", "gemini-3.1-flash-lite"],
        ),
        "xai" => (&["grok-4.7", "grok-4.6", "grok-4.5"], &[]),
        "mistral" => {
            (&["mistral-large-latest", "mistral-medium-latest"], &["mistral-small-latest"])
        }
        "deepseek" => (&["deepseek-flash", "deepseek-v4-pro"], &[]),
        "groq" => (
            &["qwen/qwen3.8-27b", "openai/gpt-oss-120b"],
            &["openai/gpt-oss-20b", "llama-3.1-8b-instant"],
        ),
        "cerebras" => (&["qwen-3.8-27b", "gpt-oss-120b"], &[]),
        "together" => (&["moonshotai/Kimi-K3", "Qwen/Qwen4-Max"], &["Qwen/Qwen3.5-9B"]),
        "fireworks" => (
            &["accounts/fireworks/models/kimi-k3", "accounts/fireworks/models/kimi-k2p5"],
            &["accounts/fireworks/models/glm-5p3-flash"],
        ),
        "openrouter" => (&["~openai/gpt-sol-latest", "openai/gpt-6.2-sol", "openrouter/auto"], &[]),
        _ => (&[], &[]),
    }
}

/// Every Provider known by name, the most used first.
pub fn known_providers() -> &'static [KnownProvider] {
    KNOWN
}

/// A Provider chosen at run time.
pub enum Connected {
    OpenAi(OpenAiCompatible),
    Anthropic(Anthropic),
    ChatGpt(ChatGpt),
}

impl Provider for Connected {
    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        match self {
            Self::OpenAi(provider) => provider.reply(request).await,
            Self::Anthropic(provider) => provider.reply(request).await,
            Self::ChatGpt(provider) => provider.reply(request).await,
        }
    }
}

/// As [`connect`], for the installation whose folder is `home`: a Provider
/// the Owner signed in to, rather than gave a key for, keeps there what lets
/// the Agent in.
pub fn connect_in(
    home: &Path,
    name: &str,
    api_key: Option<&str>,
    model: &str,
    api_root: Option<&str>,
) -> Result<Connected, String> {
    let chatgpt = KNOWN.iter().find(|known| known.speaks == Speaks::ChatGptPlan);
    match chatgpt.filter(|known| known.name == name.trim().to_lowercase()) {
        Some(known) => {
            let provider = ChatGpt::kept_in(home, model).named(known.name);
            // Its own addresses, unless it is reached somewhere else.
            let elsewhere =
                api_root.map(str::trim).filter(|root| !root.is_empty() && *root != known.api_root);
            Ok(Connected::ChatGpt(match elsewhere {
                Some(address) => provider.at(address),
                None => provider,
            }))
        }
        None => connect(name, api_key, model, api_root),
    }
}

/// The Provider called `name`, reached with `api_key` and thinking with
/// `model`. `api_root` replaces the Provider's usual address, and is the
/// only address of `custom`, any server speaking OpenAI's protocol. The text
/// of a failure is for the Owner.
pub fn connect(
    name: &str,
    api_key: Option<&str>,
    model: &str,
    api_root: Option<&str>,
) -> Result<Connected, String> {
    let name = name.trim().to_lowercase();
    let known = if name == "custom" {
        KnownProvider {
            name: "custom",
            label: "custom",
            speaks: Speaks::OpenAi,
            api_root: "",
            needs_key: false,
        }
    } else {
        *KNOWN.iter().find(|provider| provider.name == name).ok_or_else(|| {
            let names: Vec<&str> = KNOWN.iter().map(|provider| provider.name).collect();
            format!(
                "\"{name}\" is not a provider I know. Known: {}, or \"custom\" for any server \
                 speaking OpenAI's protocol.",
                names.join(", ")
            )
        })?
    };
    let api_root =
        api_root.map(str::trim).filter(|root| !root.is_empty()).unwrap_or(known.api_root);
    if api_root.is_empty() {
        return Err("A custom provider needs its address.".into());
    }
    let api_key = api_key.map(str::trim).filter(|key| !key.is_empty());
    if known.needs_key && api_key.is_none() {
        return Err(format!("{} needs an API key.", known.label));
    }
    // A local server ignores the key, but the protocol wants one sent.
    let api_key = api_key.unwrap_or("none");
    Ok(match known.speaks {
        Speaks::ChatGptPlan => {
            return Err("A ChatGPT plan is signed in to: run `orbite setup provider`.".into());
        }
        Speaks::OpenAi => {
            Connected::OpenAi(OpenAiCompatible::new(api_root, api_key, model).named(known.name))
        }
        Speaks::Anthropic => {
            let provider = Anthropic::new(api_key, model).at(api_root).named(known.name);
            // A key that is not tied to a workspace needs one named.
            Connected::Anthropic(match std::env::var("ORBITE_ANTHROPIC_WORKSPACE") {
                Ok(workspace) if !workspace.trim().is_empty() => {
                    provider.in_workspace(workspace.trim())
                }
                _ => provider,
            })
        }
    })
}
