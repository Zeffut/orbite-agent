//! Contract of the OpenAI-compatible Provider, against a fake counterpart.

use orbite::{Message, OpenAiCompatible, Provider, ProviderError};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn answer(text: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "choices": [{ "message": { "role": "assistant", "content": text } }]
    }))
}

#[tokio::test]
async fn sends_the_conversation_and_returns_the_models_answer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .and(body_json(json!({
            "model": "some-model",
            "messages": [
                { "role": "user", "content": "I live in Lyon." },
                { "role": "assistant", "content": "Noted." },
                { "role": "user", "content": "Where do I live?" },
            ]
        })))
        .respond_with(answer("Lyon."))
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(format!("{}/v1", server.uri()), "sk-test", "some-model");

    let reply = provider
        .reply(&[
            Message::from_owner("I live in Lyon."),
            Message::from_agent("Noted."),
            Message::from_owner("Where do I live?"),
        ])
        .await;

    assert_eq!(reply, Ok("Lyon.".to_string()));
}

#[tokio::test]
async fn a_refused_key_is_reported_with_the_providers_explanation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "message": "Incorrect API key provided." }
        })))
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-wrong", "some-model");

    let reply = provider.reply(&[Message::from_owner("Hi")]).await;

    assert_eq!(
        reply,
        Err(ProviderError::Rejected(
            "Incorrect API key provided.".into()
        ))
    );
}

#[tokio::test]
async fn a_server_error_is_reported_as_a_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream overloaded"))
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model");

    let reply = provider.reply(&[Message::from_owner("Hi")]).await;

    assert_eq!(
        reply,
        Err(ProviderError::Failed(
            "HTTP 503: upstream overloaded".into()
        ))
    );
}

#[tokio::test]
async fn an_answer_without_text_is_reported_as_a_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [] })))
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model");

    let reply = provider.reply(&[Message::from_owner("Hi")]).await;

    assert!(matches!(reply, Err(ProviderError::Failed(_))), "{reply:?}");
}

#[tokio::test]
async fn a_provider_that_cannot_be_reached_is_reported_as_such() {
    let provider = OpenAiCompatible::new("http://127.0.0.1:1", "sk-test", "some-model");

    let reply = provider.reply(&[Message::from_owner("Hi")]).await;

    assert!(
        matches!(reply, Err(ProviderError::Unreachable(_))),
        "{reply:?}"
    );
}
