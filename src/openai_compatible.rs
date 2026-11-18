use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Message, Provider, ProviderError, Speaker};

/// A Provider reached with an API Key over the chat-completions protocol that
/// OpenAI introduced and most Providers and local servers also speak.
pub struct OpenAiCompatible {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
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
            endpoint: format!(
                "{}/chat/completions",
                base_url.as_ref().trim_end_matches('/')
            ),
            api_key: api_key.into(),
            model: model.into(),
        }
    }
}

impl Provider for OpenAiCompatible {
    async fn reply(&self, conversation: &[Message]) -> Result<String, ProviderError> {
        let messages: Vec<Value> = conversation
            .iter()
            .map(|message| {
                let role = match message.speaker {
                    Speaker::Owner => "user",
                    Speaker::Agent => "assistant",
                };
                json!({ "role": role, "content": message.text })
            })
            .collect();

        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&json!({ "model": self.model, "messages": messages }))
            .send()
            .await
            .map_err(|error| ProviderError::Unreachable(root_cause(&error)))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| ProviderError::Failed(root_cause(&error)))?;

        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(ProviderError::Rejected(error_message(&body)));
        }
        if !status.is_success() {
            return Err(ProviderError::Failed(format!(
                "HTTP {}: {}",
                status.as_u16(),
                error_message(&body)
            )));
        }

        serde_json::from_str::<Completion>(&body)
            .ok()
            .and_then(|completion| completion.choices.into_iter().next())
            .and_then(|choice| choice.message.content)
            .ok_or_else(|| ProviderError::Failed("the answer contained no text".into()))
    }
}

#[derive(Deserialize)]
struct Completion {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
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
