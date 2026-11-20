//! Contract of the Anthropic Provider (the Messages API), against a fake counterpart.

use std::time::Duration;

use orbite::{
    ActionRequest, ActionSpec, Anthropic, Message, Provider, ProviderError, Reply, Request,
};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn answered(content: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
        "content": content, "stop_reason": "end_turn",
        "usage": { "input_tokens": 10, "output_tokens": 5 },
    }))
}

fn request<'a>(conversation: &'a [Message], actions: &'a [ActionSpec]) -> Request<'a> {
    Request { instructions: "Be brief.", conversation, actions, images: &[], light: false }
}

/// The body of the one request the fake API received.
async fn sent(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "one request was expected");
    serde_json::from_slice(&requests[0].body).unwrap()
}

async fn provider_answering(content: Value) -> (MockServer, Anthropic) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(answered(content))
        .mount(&server)
        .await;
    let provider = Anthropic::new("sk-ant-test", "claude-opus-5-5").at(server.uri());
    (server, provider)
}

#[tokio::test]
async fn sends_the_conversation_as_the_api_expects_and_returns_the_answer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", "2026-10-21"))
        .and(body_partial_json(json!({
            "model": "claude-opus-5-5",
            "system": "Be brief.",
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "I live in Lyon." }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "Noted." }] },
                { "role": "user", "content": [{ "type": "text", "text": "Where do I live?" }] },
            ]
        })))
        .respond_with(answered(json!([{ "type": "text", "text": "Lyon." }])))
        .expect(1)
        .mount(&server)
        .await;
    let provider = Anthropic::new("sk-ant-test", "claude-opus-5-5").at(server.uri());

    let reply = provider
        .reply(request(
            &[
                Message::from_owner("I live in Lyon."),
                Message::from_agent("Noted."),
                Message::from_owner("Where do I live?"),
            ],
            &[],
        ))
        .await
        .unwrap();

    assert_eq!(reply.text, "Lyon.");
    assert!(reply.actions.is_empty());
    let body = sent(&server).await;
    assert!(body["max_tokens"].as_u64().unwrap() >= 8_000, "room to answer: {body}");
    assert!(body.get("temperature").is_none() && body.get("thinking").is_none(), "{body}");
}

#[tokio::test]
async fn offers_the_actions_and_returns_the_ones_the_model_asks_for() {
    let (server, provider) = provider_answering(json!([
        { "type": "text", "text": "Let me check." },
        { "type": "tool_use", "id": "toolu_1", "name": "run_command", "input": { "command": "date" } },
    ]))
    .await;
    let actions = [ActionSpec {
        name: "run_command".into(),
        description: "Runs a shell command.".into(),
        input: json!({ "type": "object", "properties": { "command": { "type": "string" } } }),
    }];

    let reply = provider
        .reply(request(&[Message::from_owner("What time is it?")], &actions))
        .await
        .unwrap();

    assert_eq!(reply.text, "Let me check.");
    assert_eq!(
        reply.actions,
        [ActionRequest {
            id: "toolu_1".into(),
            name: "run_command".into(),
            input: json!({ "command": "date" })
        }]
    );
    assert_eq!(
        sent(&server).await["tools"],
        json!([{
            "name": "run_command",
            "description": "Runs a shell command.",
            "input_schema": { "type": "object", "properties": { "command": { "type": "string" } } },
        }])
    );
}

#[tokio::test]
async fn gives_back_every_outcome_of_a_turn_in_one_message_before_what_the_owner_adds() {
    let (server, provider) = provider_answering(json!([{ "type": "text", "text": "Done." }])).await;
    let asked = |id: &str, command: &str| ActionRequest {
        id: id.into(),
        name: "run_command".into(),
        input: json!({ "command": command }),
    };

    provider
        .reply(request(
            &[
                Message::from_owner("Check both."),
                Message::Agent {
                    text: String::new(),
                    actions: vec![asked("toolu_1", "date"), asked("toolu_2", "pwd")],
                    kept: None,
                },
                Message::Outcome { action: "toolu_1".into(), text: "Thursday".into() },
                Message::Outcome { action: "toolu_2".into(), text: "/home".into() },
                Message::from_owner("And be quick."),
                Message::Notice("A sub-agent finished.".into()),
            ],
            &[],
        ))
        .await
        .unwrap();

    assert_eq!(
        sent(&server).await["messages"],
        json!([
            { "role": "user", "content": [{ "type": "text", "text": "Check both." }] },
            { "role": "assistant", "content": [
                { "type": "tool_use", "id": "toolu_1", "name": "run_command", "input": { "command": "date" } },
                { "type": "tool_use", "id": "toolu_2", "name": "run_command", "input": { "command": "pwd" } },
            ] },
            { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1", "content": "Thursday" },
                { "type": "tool_result", "tool_use_id": "toolu_2", "content": "/home" },
                { "type": "text", "text": "And be quick." },
                { "type": "text", "text": "[Notice for you, not written by your Owner]\nA sub-agent finished." },
            ] },
        ])
    );
}

#[tokio::test]
async fn what_the_model_needs_back_as_it_was_is_kept_and_given_back_untouched() {
    // The model's reasoning comes as blocks only it can read; they must
    // return exactly as they were, around the text and the Actions.
    let thought = json!({ "type": "thinking", "thinking": "", "signature": "opaque-signature" });
    let content = json!([
        thought,
        { "type": "text", "text": "Let me check." },
        { "type": "tool_use", "id": "toolu_1", "name": "run_command", "input": { "command": "date" } },
    ]);
    let (_first, provider) = provider_answering(content.clone()).await;
    let reply =
        provider.reply(request(&[Message::from_owner("What time is it?")], &[])).await.unwrap();
    assert!(reply.kept.is_some(), "the Provider asks for its blocks to be kept");

    let (server, provider) =
        provider_answering(json!([{ "type": "text", "text": "It is noon." }])).await;
    provider
        .reply(request(
            &[
                Message::from_owner("What time is it?"),
                Message::Agent { text: reply.text, actions: reply.actions, kept: reply.kept },
                Message::Outcome { action: "toolu_1".into(), text: "12:00".into() },
            ],
            &[],
        ))
        .await
        .unwrap();

    assert_eq!(
        sent(&server).await["messages"][1],
        json!({ "role": "assistant", "content": content })
    );
}

#[tokio::test]
async fn what_another_provider_kept_is_not_sent_to_this_one() {
    let (server, provider) = provider_answering(json!([{ "type": "text", "text": "Hi." }])).await;

    provider
        .reply(request(
            &[
                Message::from_owner("Hi"),
                Message::Agent {
                    text: "Hello.".into(),
                    actions: vec![],
                    kept: Some(json!({ "someone-else": ["x"] })),
                },
                Message::from_owner("Again"),
            ],
            &[],
        ))
        .await
        .unwrap();

    assert_eq!(
        sent(&server).await["messages"][1],
        json!({ "role": "assistant", "content": [{ "type": "text", "text": "Hello." }] })
    );
}

#[tokio::test]
async fn an_image_is_sent_before_the_words_it_came_with() {
    let folder = tempfile::tempdir().unwrap();
    let photo = folder.path().join("photo.png");
    std::fs::write(&photo, [0x89, b'P', b'N', b'G']).unwrap();
    let (server, provider) =
        provider_answering(json!([{ "type": "text", "text": "A picture." }])).await;
    let conversation = [Message::from_owner("What is this?")];
    let images = [(0, photo)];

    provider
        .reply(Request {
            instructions: "",
            conversation: &conversation,
            actions: &[],
            images: &images,
            light: false,
        })
        .await
        .unwrap();

    let body = sent(&server).await;
    assert_eq!(
        body["messages"][0]["content"],
        json!([
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "iVBORw==" } },
            { "type": "text", "text": "What is this?" },
        ])
    );
    assert!(body.get("system").is_none(), "no instructions, no system prompt: {body}");
}

#[tokio::test]
async fn failures_are_told_apart() {
    for (status, headers, body, expected) in [
        (
            401,
            vec![],
            json!({ "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" } }),
            ProviderError::Rejected("invalid x-api-key".into()),
        ),
        (
            403,
            vec![],
            json!({ "type": "error", "error": { "type": "permission_error", "message": "not allowed" } }),
            ProviderError::Rejected("not allowed".into()),
        ),
        (
            429,
            vec![("retry-after", "7")],
            json!({ "type": "error", "error": { "type": "rate_limit_error", "message": "slow down" } }),
            ProviderError::Busy {
                reason: "slow down".into(),
                retry_after: Some(Duration::from_secs(7)),
            },
        ),
        (
            529,
            vec![],
            json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } }),
            ProviderError::Busy { reason: "Overloaded".into(), retry_after: None },
        ),
        (
            400,
            vec![],
            json!({ "type": "error", "error": { "type": "invalid_request_error", "message": "bad field" } }),
            ProviderError::Failed("HTTP 400: bad field".into()),
        ),
    ] {
        let server = MockServer::start().await;
        let mut response = ResponseTemplate::new(status).set_body_json(body);
        for (name, value) in headers {
            response = response.insert_header(name, value);
        }
        Mock::given(method("POST")).respond_with(response).mount(&server).await;
        let provider = Anthropic::new("sk-ant-test", "claude-opus-5-5").at(server.uri());

        let reply = provider.reply(request(&[Message::from_owner("Hi")], &[])).await;

        assert_eq!(reply, Err(expected), "for HTTP {status}");
    }
}

#[tokio::test]
async fn a_request_the_model_declines_is_reported_as_declined_with_its_reason() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "content": [], "stop_reason": "refusal",
            "stop_details": { "type": "refusal", "category": "cyber", "explanation": "This request was declined." },
        })))
        .mount(&server)
        .await;
    let provider = Anthropic::new("sk-ant-test", "claude-opus-5-5").at(server.uri());

    let reply: Result<Reply, _> = provider.reply(request(&[Message::from_owner("Hi")], &[])).await;

    assert_eq!(reply, Err(ProviderError::Declined("cyber: This request was declined.".into())));
}

#[tokio::test]
async fn an_unreachable_api_is_reported_as_such() {
    let provider = Anthropic::new("sk-ant-test", "claude-opus-5-5").at("http://127.0.0.1:1");

    let reply = provider.reply(request(&[Message::from_owner("Hi")], &[])).await;

    assert!(matches!(reply, Err(ProviderError::Unreachable(_))), "{reply:?}");
}

#[tokio::test]
async fn reports_what_the_answer_cost_counting_what_was_read_from_the_cache() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{ "type": "text", "text": "Hi." }], "stop_reason": "end_turn", "model": "claude-opus-5-5",
            "usage": {
                "input_tokens": 20, "cache_read_input_tokens": 900, "cache_creation_input_tokens": 80,
                "output_tokens": 7,
            },
        })))
        .mount(&server)
        .await;
    let provider = Anthropic::new("sk-ant-test", "claude-opus-5-5").at(server.uri());

    let usage =
        provider.reply(request(&[Message::from_owner("Hi")], &[])).await.unwrap().usage.unwrap();

    assert_eq!(
        usage,
        orbite::Usage {
            by: "anthropic".into(),
            model: "claude-opus-5-5".into(),
            input: 1000,
            cached: 900,
            output: 7
        }
    );
}

#[tokio::test]
async fn a_key_not_tied_to_a_workspace_is_used_with_the_workspace_named() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("anthropic-workspace-id", "wrkspc_123"))
        .respond_with(answered(json!([{ "type": "text", "text": "Hi." }])))
        .expect(1)
        .mount(&server)
        .await;
    let provider = Anthropic::new("sk-ant-test", "claude-opus-5-5")
        .at(server.uri())
        .in_workspace("wrkspc_123");

    assert_eq!(
        provider.reply(request(&[Message::from_owner("Hi")], &[])).await.unwrap().text,
        "Hi."
    );

    // Without one, the header is not sent at all.
    let (server, provider) = provider_answering(json!([{ "type": "text", "text": "Hi." }])).await;
    provider.reply(request(&[Message::from_owner("Hi")], &[])).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert!(!requests[0].headers.contains_key("anthropic-workspace-id"));
}
