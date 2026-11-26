//! Thinking through the server (ADR 0022): an Agent held by a server has
//! no key of its own. It asks the server, which asks the Owner's Providers
//! with the keys it holds, counts what every Agent spends against one
//! limit, and gives the answer back.

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::{ActionRequest, ActionSpec, Message, Provider, ProviderError, Reply, Request, Usage};

/// The server, as the one an Agent thinks with.
pub struct Served {
    at: String,
    token: String,
    http: reqwest::Client,
}

impl Served {
    /// Thinks through the server reached at `at`, known by `token`.
    pub fn at(at: impl Into<String>, token: impl Into<String>) -> Self {
        Self { at: at.into(), token: token.into(), http: reqwest::Client::new() }
    }
}

/// What a Provider is asked, as it travels: owned, pictures included.
pub struct Asked {
    pub instructions: String,
    pub conversation: Vec<Message>,
    pub actions: Vec<ActionSpec>,
    /// Each picture with the place in the conversation of what it came with.
    pub images: Vec<(usize, PathBuf)>,
    pub light: bool,
    /// Where the pictures were put down to be read: held for as long as
    /// this is, and removed with it.
    _kept_in: Option<tempfile::TempDir>,
}

impl Asked {
    /// As a Provider takes it.
    pub fn as_request(&self) -> Request<'_> {
        Request {
            instructions: &self.instructions,
            conversation: &self.conversation,
            actions: &self.actions,
            images: &self.images,
            light: self.light,
        }
    }

    /// What an Agent sent, read; None when it is not what one sends.
    pub fn read(sent: &Value) -> Option<Self> {
        let conversation: Vec<Message> =
            serde_json::from_value(sent["conversation"].clone()).ok()?;
        let actions = sent["actions"]
            .as_array()?
            .iter()
            .map(|action| {
                Some(ActionSpec {
                    name: action["name"].as_str()?.to_owned(),
                    description: action["description"].as_str()?.to_owned(),
                    input: action["input"].clone(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        // Pictures are put down where a Provider reads them from, under
        // names of the server's own: nothing an Agent says is a path here.
        let sent_pictures = sent["images"].as_array().cloned().unwrap_or_default();
        // No more of them than a Provider takes, none larger than one is.
        if sent_pictures.len() > 20
            || sent_pictures.iter().any(|picture| {
                picture["content"].as_str().is_none_or(|content| content.len() > 12_000_000)
            })
        {
            return None;
        }
        let (mut images, mut kept_in) = (Vec::new(), None);
        for (nth, picture) in sent_pictures.iter().enumerate() {
            let folder = match &kept_in {
                Some(folder) => folder,
                None => kept_in.insert(tempfile::tempdir().ok()?),
            };
            let ending: String = picture["ending"]
                .as_str()
                .unwrap_or("png")
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(5)
                .collect();
            let file = folder.path().join(format!("{nth}.{ending}"));
            std::fs::write(&file, STANDARD.decode(picture["content"].as_str()?).ok()?).ok()?;
            images.push((usize::try_from(picture["at"].as_u64()?).ok()?, file));
        }
        Some(Self {
            instructions: sent["instructions"].as_str()?.to_owned(),
            conversation,
            actions,
            images,
            light: sent["light"].as_bool().unwrap_or(false),
            _kept_in: kept_in,
        })
    }
}

/// An answer, or why there is none, as it travels back.
pub fn written(answered: &Result<Reply, ProviderError>) -> Value {
    match answered {
        Ok(reply) => json!({
            "text": reply.text,
            "actions": reply.actions,
            "kept": reply.kept,
            "usage": reply.usage.as_ref().map(|usage| json!({
                "by": usage.by, "model": usage.model, "input": usage.input,
                "cached": usage.cached, "output": usage.output,
            })),
        }),
        Err(problem) => {
            let (kind, reason, wait) = match problem {
                ProviderError::Rejected(reason) => ("rejected", reason, None),
                ProviderError::Unreachable(reason) => ("unreachable", reason, None),
                ProviderError::Busy { reason, retry_after } => ("busy", reason, *retry_after),
                ProviderError::Declined(reason) => ("declined", reason, None),
                ProviderError::Failed(reason) => ("failed", reason, None),
            };
            json!({ "error": kind, "reason": reason, "retry_after": wait.map(|wait| wait.as_secs_f64()) })
        }
    }
}

impl Provider for Served {
    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        let unreachable = |why: String| ProviderError::Unreachable(format!("the server: {why}"));
        let mut images = Vec::new();
        for (at, file) in request.images {
            let content =
                std::fs::read(file).map_err(|error| ProviderError::Failed(error.to_string()))?;
            let ending = file.extension().and_then(|ending| ending.to_str()).unwrap_or("png");
            images.push(json!({ "at": at, "ending": ending, "content": STANDARD.encode(content) }));
        }
        let actions: Vec<Value> = request
            .actions
            .iter()
            .map(|action| {
                json!({ "name": action.name, "description": action.description, "input": action.input })
            })
            .collect();
        let asked = json!({
            "instructions": request.instructions, "conversation": request.conversation,
            "actions": actions, "images": images, "light": request.light,
        });
        let answer = self
            .http
            .post(format!("{}/internal/think", self.at.trim_end_matches('/')))
            .header("x-orbite-agent", &self.token)
            .json(&asked)
            // Long enough for the longest answer.
            .timeout(Duration::from_secs(660))
            .send()
            .await
            .map_err(|error| unreachable(error.without_url().to_string()))?;
        let status = answer.status();
        let told: Value = answer
            .json()
            .await
            .map_err(|_| unreachable(format!("it answered what is not an answer ({status})")))?;
        if let Some(kind) = told["error"].as_str() {
            let reason = told["reason"].as_str().unwrap_or("no reason given").to_owned();
            return Err(match kind {
                "rejected" => ProviderError::Rejected(reason),
                "unreachable" => ProviderError::Unreachable(reason),
                "busy" => ProviderError::Busy {
                    reason,
                    retry_after: told["retry_after"]
                        .as_f64()
                        .filter(|wait| wait.is_finite() && (0.0..=86_400.0).contains(wait))
                        .map(Duration::from_secs_f64),
                },
                "declined" => ProviderError::Declined(reason),
                _ => ProviderError::Failed(reason),
            });
        }
        let actions: Vec<ActionRequest> = serde_json::from_value(told["actions"].clone())
            .map_err(|_| unreachable("it answered what is not an answer".into()))?;
        let usage = told["usage"].as_object().map(|usage| Usage {
            by: usage["by"].as_str().unwrap_or_default().to_owned(),
            model: usage["model"].as_str().unwrap_or_default().to_owned(),
            input: usage["input"].as_u64().unwrap_or(0),
            cached: usage["cached"].as_u64().unwrap_or(0),
            output: usage["output"].as_u64().unwrap_or(0),
        });
        Ok(Reply {
            text: told["text"].as_str().unwrap_or_default().to_owned(),
            actions,
            kept: Some(told["kept"].clone()).filter(|kept| !kept.is_null()),
            usage,
        })
    }
}
