//! The Providers known by name, and how each is reached.

use crate::{Anthropic, OpenAiCompatible, Provider, ProviderError, Reply, Request};

/// The protocol a Provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaks {
    /// The chat-completions protocol OpenAI introduced and most others took up.
    OpenAi,
    /// Anthropic's Messages API.
    Anthropic,
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

/// Every Provider known by name, the most used first.
pub fn known_providers() -> &'static [KnownProvider] {
    KNOWN
}

/// A Provider chosen at run time.
pub enum Connected {
    OpenAi(OpenAiCompatible),
    Anthropic(Anthropic),
}

impl Provider for Connected {
    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        match self {
            Self::OpenAi(provider) => provider.reply(request).await,
            Self::Anthropic(provider) => provider.reply(request).await,
        }
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
        Speaks::OpenAi => Connected::OpenAi(OpenAiCompatible::new(api_root, api_key, model)),
        Speaks::Anthropic => Connected::Anthropic(Anthropic::new(api_key, model).at(api_root)),
    })
}
