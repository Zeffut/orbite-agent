use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::attachments::image_type;
use crate::{ActionRequest, Message, Provider, ProviderError, Reply, Request};

/// The version of the Messages API this is written for.
const API_VERSION: &str = "2026-10-21";

/// Room for an answer. Requests are not streamed, so this stays well under
/// what would take longer than a request may.
const MAX_TOKENS: u32 = 16_000;

/// Under this name the model's own content is kept in the Conversation.
const KEPT_AS: &str = "anthropic";

/// Anthropic's models, reached with an API Key over the Messages API.
pub struct Anthropic {
    http: reqwest::Client,
    api_root: String,
    api_key: String,
    model: String,
}

impl Anthropic {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_root: "https://api.anthropic.com".into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    /// Talks to another server speaking the Messages API.
    pub fn at(mut self, api_root: impl Into<String>) -> Self {
        self.api_root = api_root.into().trim_end_matches('/').to_owned();
        self
    }
}

impl Provider for Anthropic {
    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "messages": messages(&request),
            // What does not change from one turn to the next is read from
            // the cache rather than paid for again.
            "cache_control": { "type": "ephemeral" },
        });
        if !request.instructions.is_empty() {
            body["system"] = json!(request.instructions);
        }
        if !request.actions.is_empty() {
            body["tools"] = request
                .actions
                .iter()
                .map(|action| {
                    json!({
                        "name": action.name,
                        "description": action.description,
                        "input_schema": action.input,
                    })
                })
                .collect();
        }

        let response = self
            .http
            .post(format!("{}/v1/messages", self.api_root))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .timeout(Duration::from_secs(600))
            .json(&body)
            .send()
            .await
            .map_err(|error| ProviderError::Unreachable(root_cause(&error)))?;

        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok()?.trim().parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .map(Duration::from_secs_f64);
        let body =
            response.text().await.map_err(|error| ProviderError::Failed(root_cause(&error)))?;
        match status {
            200..=299 => {}
            401 | 403 => return Err(ProviderError::Rejected(error_message(&body))),
            429 | 500..=599 => {
                return Err(ProviderError::Busy { reason: error_message(&body), retry_after });
            }
            _ => {
                return Err(ProviderError::Failed(format!(
                    "HTTP {status}: {}",
                    error_message(&body)
                )));
            }
        }

        let answer: Answer = serde_json::from_str(&body)
            .map_err(|_| ProviderError::Failed("the answer was not understood".into()))?;
        if answer.stop_reason.as_deref() == Some("refusal") {
            let details = answer.stop_details.unwrap_or_default();
            let category = details["category"].as_str().unwrap_or("no category given");
            let explanation = details["explanation"].as_str().unwrap_or_default();
            return Err(ProviderError::Declined(
                format!("{category}: {explanation}").trim_end_matches(": ").to_owned(),
            ));
        }
        let mut text = String::new();
        let mut actions = Vec::new();
        for block in &answer.content {
            match block["type"].as_str() {
                Some("text") => text.push_str(block["text"].as_str().unwrap_or_default()),
                Some("tool_use") => actions.push(ActionRequest {
                    id: block["id"].as_str().unwrap_or_default().to_owned(),
                    name: block["name"].as_str().unwrap_or_default().to_owned(),
                    input: block["input"].clone(),
                }),
                // Reasoning and the like: kept below, not for Orbite to read.
                _ => {}
            }
        }
        if text.is_empty() && actions.is_empty() {
            return Err(ProviderError::Failed("the answer contained no text".into()));
        }
        // Everything the model wrote goes back to it exactly as it was.
        let kept = Some(json!({ KEPT_AS: answer.content }));
        Ok(Reply { text, actions, kept })
    }
}

/// The Conversation in the API's own shape: turns alternate, each made of
/// blocks, and every Outcome of a turn comes back in the one message that
/// follows it.
fn messages(request: &Request<'_>) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    for (place, message) in request.conversation.iter().enumerate() {
        let (role, mut blocks) = match message {
            Message::Owner(text) => ("user", vec![json!({ "type": "text", "text": text })]),
            Message::Notice(text) => (
                "user",
                vec![json!({
                    "type": "text",
                    "text": format!("[Notice for you, not written by your Owner]\n{text}"),
                })],
            ),
            Message::Outcome { action, text } => (
                "user",
                vec![json!({ "type": "tool_result", "tool_use_id": action, "content": text })],
            ),
            Message::Agent { text, actions, kept } => {
                let own = kept.as_ref().and_then(|kept| kept[KEPT_AS].as_array());
                let blocks = match own {
                    Some(blocks) => blocks.clone(),
                    None => {
                        let mut blocks = Vec::new();
                        if !text.is_empty() {
                            blocks.push(json!({ "type": "text", "text": text }));
                        }
                        blocks.extend(actions.iter().map(|action| {
                            json!({ "type": "tool_use", "id": action.id, "name": action.name, "input": action.input })
                        }));
                        blocks
                    }
                };
                ("assistant", blocks)
            }
        };
        // Images come before the words they were sent with.
        let images: Vec<Value> = request
            .images
            .iter()
            .filter(|(at, _)| *at == place && role == "user")
            .filter_map(|(_, image)| {
                let data =
                    base64::engine::general_purpose::STANDARD.encode(std::fs::read(image).ok()?);
                Some(json!({
                    "type": "image",
                    "source": { "type": "base64", "media_type": image_type(image)?, "data": data },
                }))
            })
            .collect();
        blocks.splice(0..0, images);
        if blocks.is_empty() {
            continue;
        }
        match messages.last_mut() {
            Some(last) if last["role"] == role => {
                last["content"].as_array_mut().expect("content is a list of blocks").extend(blocks);
            }
            _ => messages.push(json!({ "role": role, "content": blocks })),
        }
    }
    messages
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    content: Vec<Value>,
    stop_reason: Option<String>,
    stop_details: Option<Value>,
}

/// The API's own explanation when the body carries one, else the body.
fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.trim().to_owned())
}

/// The innermost cause, which is the part a person can act on.
fn root_cause(error: &dyn std::error::Error) -> String {
    let mut cause = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    cause.to_string()
}
