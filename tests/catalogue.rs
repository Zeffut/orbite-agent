//! Choosing a Provider by its name.

use orbite::{Message, Provider, Request, connect, known_providers};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn hi() -> Request<'static> {
    static HI: std::sync::LazyLock<Vec<Message>> =
        std::sync::LazyLock::new(|| vec![Message::from_owner("Hi")]);
    Request { instructions: "", conversation: &HI, actions: &[], images: &[], light: false }
}

#[test]
fn the_providers_people_use_are_known_by_name() {
    let names: Vec<&str> = known_providers().iter().map(|provider| provider.name).collect();
    for expected in [
        "openai",
        "anthropic",
        "google",
        "groq",
        "openrouter",
        "mistral",
        "deepseek",
        "xai",
        "ollama",
    ] {
        assert!(names.contains(&expected), "{expected} is missing from {names:?}");
    }
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "a name is listed twice");
    for provider in known_providers() {
        assert!(provider.api_root.starts_with("http"), "{}", provider.name);
        assert!(!provider.label.is_empty());
    }
}

#[tokio::test]
async fn a_provider_speaking_openais_protocol_is_reached_that_way() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer key-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "role": "assistant", "content": "Hello." } }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider = connect("groq", Some("key-1"), "some-model", Some(&server.uri())).unwrap();

    assert_eq!(provider.reply(hi()).await.unwrap().text, "Hello.");
}

#[tokio::test]
async fn anthropic_is_reached_through_its_own_api() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "key-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{ "type": "text", "text": "Hello." }], "stop_reason": "end_turn"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider =
        connect("anthropic", Some("key-1"), "claude-opus-5-5", Some(&server.uri())).unwrap();

    assert_eq!(provider.reply(hi()).await.unwrap().text, "Hello.");
}

#[test]
fn a_local_provider_needs_no_key_and_any_other_does() {
    assert!(connect("ollama", None, "some-model", None).is_ok());

    let Err(missing) = connect("openai", None, "some-model", None) else {
        panic!("a key was not asked for")
    };
    assert!(missing.contains("API key"), "{missing}");
}

#[test]
fn an_endpoint_of_ones_own_is_accepted_and_must_be_given() {
    assert!(
        connect("custom", Some("key"), "some-model", Some("https://llm.example.org/v1")).is_ok()
    );

    let Err(missing) = connect("custom", Some("key"), "some-model", None) else { panic!() };
    assert!(missing.contains("address"), "{missing}");
}

#[test]
fn an_unknown_name_is_answered_with_the_known_ones() {
    let Err(unknown) = connect("opnai", Some("key"), "some-model", None) else { panic!() };

    assert!(
        unknown.contains("opnai") && unknown.contains("openai") && unknown.contains("anthropic"),
        "{unknown}"
    );
}
