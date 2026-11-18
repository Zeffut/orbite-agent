//! Contract of the OpenAI-compatible Provider, against a fake counterpart.

use orbite::{
    ActionRequest, ActionSpec, Message, OpenAiCompatible, Provider, ProviderError, Reply, Request,
};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The answer alone: what it cost is checked on its own.
fn without_cost(reply: Result<Reply, ProviderError>) -> Result<Reply, ProviderError> {
    reply.map(|reply| Reply { usage: None, ..reply })
}

fn said(text: &str) -> Reply {
    Reply { text: text.to_string(), actions: Vec::new(), kept: None, usage: None }
}

/// The smallest request: the Owner says hi.
fn hi() -> Request<'static> {
    static HI: std::sync::LazyLock<Vec<Message>> =
        std::sync::LazyLock::new(|| vec![Message::from_owner("Hi")]);
    Request { instructions: "", conversation: &HI, actions: &[], images: &[], light: false }
}

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
                { "role": "system", "content": "Be brief." },
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
        .reply(Request {
            instructions: "Be brief.",
            conversation: &[
                Message::from_owner("I live in Lyon."),
                Message::from_agent("Noted."),
                Message::from_owner("Where do I live?"),
            ],
            actions: &[],
            images: &[],
            light: false,
        })
        .await;

    assert_eq!(without_cost(reply), Ok(said("Lyon.")));
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

    let reply = provider.reply(hi()).await;

    assert_eq!(reply, Err(ProviderError::Rejected("Incorrect API key provided.".into())));
}

#[tokio::test]
async fn a_rate_limit_is_reported_as_busy_with_the_wait_the_provider_asks_for() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "2.5")
                .set_body_json(json!({ "error": { "message": "Rate limit reached." } })),
        )
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model");

    let reply = provider.reply(hi()).await;

    assert_eq!(
        reply,
        Err(ProviderError::Busy {
            reason: "Rate limit reached.".into(),
            retry_after: Some(std::time::Duration::from_millis(2500)),
        })
    );
}

#[tokio::test]
async fn an_overloaded_provider_is_reported_as_busy() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream overloaded"))
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model");

    let reply = provider.reply(hi()).await;

    assert_eq!(
        reply,
        Err(ProviderError::Busy {
            reason: "HTTP 503: upstream overloaded".into(),
            retry_after: None
        })
    );
}

#[tokio::test]
async fn a_request_the_provider_cannot_accept_is_reported_as_a_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": { "message": "Unknown model." }
        })))
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model");

    let reply = provider.reply(hi()).await;

    assert_eq!(reply, Err(ProviderError::Failed("HTTP 400: Unknown model.".into())));
}

#[tokio::test]
async fn an_answer_without_text_is_reported_as_a_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [] })))
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model");

    let reply = provider.reply(hi()).await;

    assert!(matches!(reply, Err(ProviderError::Failed(_))), "{reply:?}");
}

#[tokio::test]
async fn a_provider_that_cannot_be_reached_is_reported_as_such() {
    let provider = OpenAiCompatible::new("http://127.0.0.1:1", "sk-test", "some-model");

    let reply = provider.reply(hi()).await;

    assert!(matches!(reply, Err(ProviderError::Unreachable(_))), "{reply:?}");
}

#[tokio::test]
async fn offers_the_actions_and_returns_the_ones_the_model_asks_for() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_json(json!({
            "model": "some-model",
            "messages": [{ "role": "user", "content": "What time is it?" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "run_command",
                    "description": "Runs a shell command.",
                    "parameters": { "type": "object", "properties": { "command": { "type": "string" } } },
                }
            }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "run_command", "arguments": "{\"command\":\"date\"}" }
                }]
            } }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model");

    let reply = provider
        .reply(Request {
            instructions: "",
            conversation: &[Message::from_owner("What time is it?")],
            actions: &[ActionSpec {
                name: "run_command".into(),
                description: "Runs a shell command.".into(),
                input: json!({ "type": "object", "properties": { "command": { "type": "string" } } }),
            }], images: &[],
            light: false,
        })
        .await;

    assert_eq!(
        without_cost(reply),
        Ok(Reply {
            text: String::new(),
            actions: vec![ActionRequest {
                id: "call_1".into(),
                name: "run_command".into(),
                input: json!({ "command": "date" }),
            }],
            kept: None,
            usage: None,
        })
    );
}

#[tokio::test]
async fn sends_back_the_actions_asked_for_and_what_came_of_them() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_json(json!({
            "model": "some-model",
            "messages": [
                { "role": "user", "content": "What time is it?" },
                { "role": "assistant", "content": "Let me check.", "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "run_command", "arguments": "{\"command\":\"date\"}" }
                }] },
                { "role": "tool", "tool_call_id": "call_1", "content": "Thu Nov 19 10:00:00 UTC 2026" },
            ]
        })))
        .respond_with(answer("It is 10:00."))
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model");

    let reply = provider
        .reply(Request {
            instructions: "",
            conversation: &[
                Message::from_owner("What time is it?"),
                Message::Agent {
                    text: "Let me check.".into(),
                    actions: vec![ActionRequest {
                        id: "call_1".into(),
                        name: "run_command".into(),
                        input: json!({ "command": "date" }),
                    }],
                    kept: None,
                },
                Message::Outcome {
                    action: "call_1".into(),
                    text: "Thu Nov 19 10:00:00 UTC 2026".into(),
                },
            ],
            actions: &[],
            images: &[],
            light: false,
        })
        .await;

    assert_eq!(without_cost(reply), Ok(said("It is 10:00.")));
}

#[tokio::test]
async fn action_input_that_is_not_valid_json_is_reported_as_a_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "role": "assistant", "content": null, "tool_calls": [{
                "id": "call_1", "type": "function",
                "function": { "name": "run_command", "arguments": "{not json" }
            }] } }]
        })))
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model");

    let reply = provider.reply(hi()).await;

    assert!(matches!(reply, Err(ProviderError::Failed(_))), "{reply:?}");
}

#[tokio::test]
async fn an_image_is_sent_with_the_message_it_came_with() {
    let folder = tempfile::tempdir().unwrap();
    let photo = folder.path().join("photo.png");
    std::fs::write(&photo, [0x89, b'P', b'N', b'G']).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_json(json!({
            "model": "some-model",
            "messages": [
                { "role": "user", "content": "Earlier." },
                { "role": "assistant", "content": "Yes." },
                { "role": "user", "content": [
                    { "type": "text", "text": "What is this?" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,iVBORw==" } },
                ] },
            ]
        })))
        .respond_with(answer("A picture."))
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model");

    let reply = provider
        .reply(Request {
            instructions: "",
            conversation: &[
                Message::from_owner("Earlier."),
                Message::from_agent("Yes."),
                Message::from_owner("What is this?"),
            ],
            actions: &[],
            images: &[(2, photo)],
            light: false,
        })
        .await;

    assert_eq!(without_cost(reply), Ok(said("A picture.")));
}

#[tokio::test]
async fn reports_what_the_answer_cost_when_the_provider_says() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "role": "assistant", "content": "Hi." } }],
            "usage": { "prompt_tokens": 1000, "completion_tokens": 7, "prompt_tokens_details": { "cached_tokens": 900 } },
        })))
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(server.uri(), "sk-test", "some-model").named("groq");

    let usage = provider.reply(hi()).await.unwrap().usage.unwrap();

    assert_eq!(
        usage,
        orbite::Usage {
            by: "groq".into(),
            model: "some-model".into(),
            input: 1000,
            cached: 900,
            output: 7
        }
    );
}
