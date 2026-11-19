use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::attachments::image_type;
use crate::{ActionRequest, Message, Provider, ProviderError, Reply, Request, Usage};

/// A Provider reached with an API Key over the chat-completions protocol that
/// OpenAI introduced and most Providers and local servers also speak.
pub struct OpenAiCompatible {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
    /// The name it was connected under, said with what each answer cost.
    name: String,
}

impl OpenAiCompatible {
    /// `base_url` is the Provider's API root, such as `https://api.openai.com/v1`.
    pub fn new(
        base_url: impl AsRef<str>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: format!("{}/chat/completions", base_url.as_ref().trim_end_matches('/')),
            api_key: api_key.into(),
            model: model.into(),
            name: "openai-compatible".into(),
        }
    }

    /// The name this Provider goes by in what the Owner is shown.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl Provider for OpenAiCompatible {
    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        let mut messages = Vec::new();
        if !request.instructions.is_empty() {
            messages.push(json!({ "role": "system", "content": request.instructions }));
        }
        for (place, message) in request.conversation.iter().enumerate() {
            let mut written = wire_message(message);
            // A message with images becomes a list of parts: its text, then each image.
            let mut parts = Vec::new();
            for (_, image) in request.images.iter().filter(|(at, _)| *at == place) {
                let (Some(kind), Ok(bytes)) = (image_type(image), std::fs::read(image)) else {
                    continue;
                };
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:{kind};base64,{encoded}") },
                }));
            }
            if !parts.is_empty() && written["role"] == "user" {
                parts.insert(0, json!({ "type": "text", "text": written["content"] }));
                written["content"] = Value::Array(parts);
            }
            messages.push(written);
        }
        let mut body = json!({ "model": self.model, "messages": messages });
        if !request.actions.is_empty() {
            body["tools"] = request
                .actions
                .iter()
                .map(|action| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": action.name,
                            "description": action.description,
                            "parameters": action.input,
                        }
                    })
                })
                .collect();
        }

        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| ProviderError::Unreachable(root_cause(&error)))?;

        let status = response.status();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok()?.trim().parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .map(Duration::from_secs_f64);
        let body =
            response.text().await.map_err(|error| ProviderError::Failed(root_cause(&error)))?;

        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(ProviderError::Rejected(error_message(&body)));
        }
        if status.as_u16() == 429 {
            return Err(ProviderError::Busy { reason: error_message(&body), retry_after });
        }
        if status.is_server_error() {
            let reason = format!("HTTP {}: {}", status.as_u16(), error_message(&body));
            return Err(ProviderError::Busy { reason, retry_after });
        }
        // The model wrote an Action request that does not parse: it was not
        // asked anything wrong, and asking again usually gets a sound one.
        if status.as_u16() == 400
            && (body.contains("\"tool_use_failed\"") || body.contains("\"failed_generation\""))
        {
            return Err(ProviderError::Busy { reason: error_message(&body), retry_after: None });
        }
        if !status.is_success() {
            return Err(ProviderError::Failed(format!(
                "HTTP {}: {}",
                status.as_u16(),
                error_message(&body)
            )));
        }

        let completion = serde_json::from_str::<Completion>(&body)
            .map_err(|_| ProviderError::Failed("the answer was not understood".into()))?;
        let count = |value: &Value| value.as_u64().unwrap_or(0);
        let usage = Some(Usage {
            by: self.name.clone(),
            model: self.model.clone(),
            input: count(&completion.usage["prompt_tokens"]),
            cached: count(&completion.usage["prompt_tokens_details"]["cached_tokens"]),
            output: count(&completion.usage["completion_tokens"]),
        });
        let message = completion
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message)
            .ok_or_else(|| ProviderError::Failed("the answer was not understood".into()))?;
        let text = message.content.unwrap_or_default();
        let actions = message
            .tool_calls
            .into_iter()
            .map(|call| {
                let input = serde_json::from_str(&call.function.arguments).map_err(|_| {
                    ProviderError::Failed(format!(
                        "the model gave unreadable input for {}",
                        call.function.name
                    ))
                })?;
                Ok(ActionRequest { id: call.id, name: call.function.name, input })
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        if text.is_empty() && actions.is_empty() {
            return Err(ProviderError::Failed("the answer contained no text".into()));
        }
        Ok(Reply { text, actions, kept: None, usage })
    }
}

/// A Conversation entry in the protocol's own shape.
fn wire_message(message: &Message) -> Value {
    match message {
        Message::Owner(text) => json!({ "role": "user", "content": text }),
        Message::Agent { text, actions, .. } if actions.is_empty() => {
            json!({ "role": "assistant", "content": text })
        }
        Message::Agent { text, actions, .. } => {
            let calls: Vec<Value> = actions
                .iter()
                .map(|action| {
                    json!({
                        "id": action.id,
                        "type": "function",
                        "function": { "name": action.name, "arguments": action.input.to_string() },
                    })
                })
                .collect();
            json!({ "role": "assistant", "content": text, "tool_calls": calls })
        }
        Message::Notice(text) => json!({
            "role": "user",
            "content": format!("[Notice for you, not written by your Owner]\n{text}"),
        }),
        Message::Outcome { action, text } => {
            json!({ "role": "tool", "tool_call_id": action, "content": text })
        }
    }
}

#[derive(Deserialize)]
struct Completion {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Value,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
}

#[derive(Deserialize)]
struct ToolCall {
    id: String,
    function: CalledFunction,
}

#[derive(Deserialize)]
struct CalledFunction {
    name: String,
    arguments: String,
}

/// The Provider's own explanation when the body carries one, else the body.
fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.trim().to_owned())
}

/// The innermost cause, which is the part a person can act on
/// ("connection refused" rather than "error sending request").
fn root_cause(error: &dyn std::error::Error) -> String {
    let mut cause = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    cause.to_string()
}
