//! The Agent thinking with its Owner's ChatGPT plan: signing in by a link
//! and the address the browser ends on, then asking OpenAI as that Owner.
//! The fake below answers as OpenAI's documentation says its servers do.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use orbite::{ActionSpec, ChatGpt, Message, Provider, ProviderError, Request, Secrets};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ISSUED: &str = "oaiapp_issued-to-this-agent";
const ALL_SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

fn far_from_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 86_400
}

/// An identity token as OpenAI would sign one; its signature is not one.
fn identity(client: &str, nonce: &str) -> String {
    let part = |value: Value| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string().as_bytes())
    };
    format!(
        "{}.{}.{}",
        part(json!({ "alg": "RS256" })),
        part(json!({
            "iss": "https://auth.openai.com", "aud": client, "exp": far_from_now(),
            "nonce": nonce, "email": "owner@example.org",
        })),
        "signature"
    )
}

fn tokens(access: &str, refresh: &str, nonce: &str, lasts: u64) -> Value {
    json!({
        "access_token": access, "refresh_token": refresh, "id_token": identity(ISSUED, nonce),
        "token_type": "Bearer", "expires_in": lasts, "scope": ALL_SCOPES,
    })
}

/// What the Owner's browser asked for by following the link.
fn asked_in(link: &str) -> HashMap<String, String> {
    url::Url::parse(link).unwrap().query_pairs().into_owned().collect()
}

/// The address the browser ends on once the Owner agreed.
fn ended_on(link: &str) -> String {
    let asked = asked_in(link);
    format!(
        "{}?code=code-for-the-test&scope=openid&state={}&client_id={ISSUED}",
        asked["redirect_uri"], asked["state"]
    )
}

/// The answer to a question, as the stream of events OpenAI sends.
fn stream(events: &[Value]) -> ResponseTemplate {
    let written: String = events
        .iter()
        .map(|event| format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap()))
        .collect();
    ResponseTemplate::new(200).set_body_raw(written, "text/event-stream")
}

fn saying(text: &str) -> ResponseTemplate {
    stream(&[
        json!({ "type": "response.output_text.delta", "delta": text }),
        json!({ "type": "response.completed", "response": { "usage": {
            "input_tokens": 120, "output_tokens": 7,
            "input_tokens_details": { "cached_tokens": 100 },
        } } }),
    ])
}

/// OpenAI's sign-in, exchanging the test's code for tokens when the form
/// carries what the documentation asks for. `nonce` is the one of the link.
async fn signing_in(server: &MockServer, nonce: &str, lasts: u64) {
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=code-for-the-test"))
        .and(body_string_contains(format!("client_id={ISSUED}")))
        .and(body_string_contains("resource=https%3A%2F%2Fapi.openai.com%2Fv1"))
        .and(body_string_contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tokens(
            "access-first",
            "refresh-first",
            nonce,
            lasts,
        )))
        .mount(server)
        .await;
}

/// A ChatGPT plan signed in to, in a new home.
async fn signed_in(server: &MockServer, lasts: u64) -> (tempfile::TempDir, ChatGpt) {
    let home = tempfile::tempdir().unwrap();
    let chatgpt = ChatGpt::kept_in(home.path(), "gpt-test").at(server.uri());
    let begun = chatgpt.sign_in().unwrap();
    signing_in(server, &asked_in(begun.link())["nonce"], lasts).await;
    chatgpt.signed_in_at(&begun, &ended_on(begun.link())).await.unwrap();
    (home, chatgpt)
}

fn question<'a>(conversation: &'a [Message], actions: &'a [ActionSpec]) -> Request<'a> {
    Request { instructions: "Be brief.", conversation, actions, images: &[], light: false }
}

#[tokio::test]
async fn the_link_asks_openai_for_what_its_documentation_says() {
    let home = tempfile::tempdir().unwrap();
    let chatgpt = ChatGpt::kept_in(home.path(), "gpt-test");

    let begun = chatgpt.sign_in().unwrap();

    assert!(begun.link().starts_with("https://auth.openai.com/api/accounts/authorize?"));
    let asked = asked_in(begun.link());
    assert_eq!(asked["client_id"], "dynamic_agent_client");
    assert_eq!(asked["agent_name_hint"], "Orbite");
    assert_eq!(asked["response_type"], "code");
    assert_eq!(asked["scope"], ALL_SCOPES);
    assert_eq!(asked["resource"], "https://api.openai.com/v1");
    assert_eq!(asked["redirect_uri"], "http://127.0.0.1:1455/auth/callback");
    assert_eq!(asked["code_challenge_method"], "S256");
    assert!(asked["ext_agent_host_id"].starts_with("urn:uuid:"));
    assert_eq!(asked["ext_agent_host_id"].len(), "urn:uuid:".len() + 36);
    assert!(asked["state"].len() >= 32 && asked["nonce"].len() >= 32);

    // This machine keeps the same name with OpenAI from one link to the next.
    let again = asked_in(chatgpt.sign_in().unwrap().link());
    assert_eq!(again["ext_agent_host_id"], asked["ext_agent_host_id"]);
    assert_ne!(again["state"], asked["state"]);
}

#[tokio::test]
async fn the_owner_signs_in_and_the_agent_thinks_with_their_plan() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let chatgpt = ChatGpt::kept_in(home.path(), "gpt-test").at(server.uri());
    let begun = chatgpt.sign_in().unwrap();
    let asked = asked_in(begun.link());
    // Only who holds what the link was made from can finish the sign-in.
    let proved = Arc::new(Mutex::new(String::new()));
    let noted = proved.clone();
    let nonce = asked["nonce"].clone();
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .respond_with(move |request: &wiremock::Request| {
            let form: HashMap<String, String> =
                url::form_urlencoded::parse(&request.body).into_owned().collect();
            *noted.lock().unwrap() = form["code_verifier"].clone();
            ResponseTemplate::new(200).set_body_json(tokens(
                "access-first",
                "refresh-first",
                &nonce,
                3600,
            ))
        })
        .mount(&server)
        .await;
    let sent = Arc::new(Mutex::new(Value::Null));
    let seen = sent.clone();
    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(header("authorization", "Bearer access-first"))
        .respond_with(move |request: &wiremock::Request| {
            *seen.lock().unwrap() = serde_json::from_slice(&request.body).unwrap();
            stream(&[
                json!({ "type": "response.output_text.delta", "delta": "Let me " }),
                json!({ "type": "response.output_text.delta", "delta": "look." }),
                json!({ "type": "response.output_item.done", "item": {
                    "type": "function_call", "id": "fc_1", "call_id": "call_1",
                    "namespace": "orbite", "name": "look_up",
                    "arguments": "{\"word\":\"orbit\"}",
                } }),
                json!({ "type": "response.completed", "response": { "usage": {
                    "input_tokens": 120, "output_tokens": 7,
                    "input_tokens_details": { "cached_tokens": 100 },
                } } }),
            ])
        })
        .mount(&server)
        .await;

    let who = chatgpt.signed_in_at(&begun, &ended_on(begun.link())).await.unwrap();
    assert_eq!(who, "owner@example.org");
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(proved.lock().unwrap().as_bytes()));
    assert_eq!(challenge, asked["code_challenge"]);

    let conversation = [
        Message::from_owner("What is an orbit?"),
        Message::Agent {
            text: "I will check.".into(),
            actions: vec![orbite::ActionRequest {
                id: "call_0".into(),
                name: "look_up".into(),
                input: json!({ "word": "orbit" }),
            }],
            kept: None,
        },
        Message::Outcome { action: "call_0".into(), text: "A path around a body.".into() },
        Message::Notice("A routine ended.".into()),
    ];
    let actions = [ActionSpec {
        name: "look_up".into(),
        description: "Looks a word up.".into(),
        input: json!({ "type": "object", "properties": { "word": { "type": "string" } } }),
    }];
    let reply = chatgpt.reply(question(&conversation, &actions)).await.unwrap();

    assert_eq!(reply.text, "Let me look.");
    assert_eq!(reply.actions.len(), 1);
    assert_eq!(reply.actions[0].id, "call_1");
    assert_eq!(reply.actions[0].name, "look_up");
    assert_eq!(reply.actions[0].input, json!({ "word": "orbit" }));
    let usage = reply.usage.unwrap();
    assert_eq!((usage.input, usage.cached, usage.output), (120, 100, 7));
    assert_eq!(usage.model, "gpt-test");

    let sent = sent.lock().unwrap().clone();
    assert_eq!(sent["model"], "gpt-test");
    assert_eq!(sent["store"], false);
    assert_eq!(sent["stream"], true);
    assert_eq!(sent["instructions"], "Be brief.");
    // Nothing OpenAI refuses on this path.
    for refused in ["temperature", "max_output_tokens", "previous_response_id", "metadata", "user"]
    {
        assert!(sent.get(refused).is_none(), "{refused} is refused on this path");
    }
    assert_eq!(
        sent["input"],
        json!([
            { "role": "user", "content": "What is an orbit?" },
            { "role": "assistant", "content": "I will check." },
            { "type": "function_call", "call_id": "call_0", "namespace": "orbite",
              "name": "look_up", "arguments": "{\"word\":\"orbit\"}" },
            { "type": "function_call_output", "call_id": "call_0",
              "output": "A path around a body." },
            { "role": "user",
              "content": "[Notice for you, not written by your Owner]\nA routine ended." },
        ])
    );
    // Actions are grouped under one name, as this path wants them.
    assert_eq!(sent["tools"].as_array().unwrap().len(), 1);
    assert_eq!(sent["tools"][0]["type"], "namespace");
    assert_eq!(sent["tools"][0]["name"], "orbite");
    assert_eq!(
        sent["tools"][0]["tools"],
        json!([{
            "type": "function", "name": "look_up", "description": "Looks a word up.",
            "parameters": { "type": "object", "properties": { "word": { "type": "string" } } },
        }])
    );

    // What lets the Agent in is sealed, and nowhere else in its home.
    let kept = Secrets::kept_in(home.path()).get("ORBITE_CHATGPT").unwrap().unwrap();
    assert!(kept.contains("refresh-first"));
    let config = std::fs::read_to_string(home.path().join("config.json")).unwrap_or_default();
    assert!(!config.contains("refresh-first"));
}

#[tokio::test]
async fn an_address_that_does_not_answer_the_link_is_refused() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let chatgpt = ChatGpt::kept_in(home.path(), "gpt-test").at(server.uri());
    let begun = chatgpt.sign_in().unwrap();
    signing_in(&server, &asked_in(begun.link())["nonce"], 3600).await;

    let forged = format!(
        "http://127.0.0.1:1455/auth/callback?code=code-for-the-test&state=another&client_id={ISSUED}"
    );
    let refused = chatgpt.signed_in_at(&begun, &forged).await.unwrap_err();
    assert!(refused.contains("not the answer to the link"), "{refused}");

    let declined = format!(
        "http://127.0.0.1:1455/auth/callback?error=access_denied&state={}",
        asked_in(begun.link())["state"]
    );
    let refused = chatgpt.signed_in_at(&begun, &declined).await.unwrap_err();
    assert!(refused.contains("not granted"), "{refused}");

    let refused = chatgpt.signed_in_at(&begun, "I do not know").await.unwrap_err();
    assert!(refused.contains("not an address"), "{refused}");
    assert!(!chatgpt.is_signed_in());
}

#[tokio::test]
async fn a_sign_in_that_does_not_allow_the_plan_to_be_used_is_refused() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let chatgpt = ChatGpt::kept_in(home.path(), "gpt-test").at(server.uri());
    let begun = chatgpt.sign_in().unwrap();
    let mut granted = tokens("access-first", "refresh-first", &asked_in(begun.link())["nonce"], 60);
    granted["scope"] = json!("openid profile email offline_access");
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(granted))
        .mount(&server)
        .await;

    let refused = chatgpt.signed_in_at(&begun, &ended_on(begun.link())).await.unwrap_err();

    assert!(refused.contains("plan"), "{refused}");
    assert!(!chatgpt.is_signed_in());
}

#[tokio::test]
async fn an_identity_made_for_another_sign_in_is_refused() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let chatgpt = ChatGpt::kept_in(home.path(), "gpt-test").at(server.uri());
    let begun = chatgpt.sign_in().unwrap();
    signing_in(&server, "the-nonce-of-another-link", 3600).await;

    let refused = chatgpt.signed_in_at(&begun, &ended_on(begun.link())).await.unwrap_err();

    assert!(refused.contains("identity"), "{refused}");
    assert!(!chatgpt.is_signed_in());
}

#[tokio::test]
async fn what_lets_the_agent_in_is_renewed_before_it_runs_out_and_each_renewal_replaces_the_last() {
    let server = MockServer::start().await;
    // Lasting no time at all: every question renews first.
    let (home, chatgpt) = signed_in(&server, 0).await;
    for (old, new) in [("first", "second"), ("second", "third")] {
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains(format!("refresh_token=refresh-{old}")))
            .and(body_string_contains(format!("client_id={ISSUED}")))
            .and(body_string_contains("resource=https%3A%2F%2Fapi.openai.com%2Fv1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": format!("access-{new}"),
                "refresh_token": format!("refresh-{new}"),
                "token_type": "Bearer", "expires_in": 0, "scope": ALL_SCOPES,
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .and(header("authorization", format!("Bearer access-{new}").as_str()))
            .respond_with(saying(new))
            .mount(&server)
            .await;
    }
    let conversation = [Message::from_owner("Hello")];

    assert_eq!(chatgpt.reply(question(&conversation, &[])).await.unwrap().text, "second");
    // Another program of the same installation takes over where this one was.
    let another = ChatGpt::kept_in(home.path(), "gpt-test").at(server.uri());
    assert_eq!(another.reply(question(&conversation, &[])).await.unwrap().text, "third");
}

#[tokio::test]
async fn an_answer_that_openai_refuses_as_stale_is_asked_again_once_renewed() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 3600).await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .and(body_string_contains("refresh_token=refresh-first"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-second", "refresh_token": "refresh-second",
            "token_type": "Bearer", "expires_in": 3600, "scope": ALL_SCOPES,
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(header("authorization", "Bearer access-second"))
        .respond_with(saying("Hello again."))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "detail": "Unauthorized" })))
        .mount(&server)
        .await;
    let conversation = [Message::from_owner("Hello")];

    let reply = chatgpt.reply(question(&conversation, &[])).await.unwrap();

    assert_eq!(reply.text, "Hello again.");
}

#[tokio::test]
async fn a_sign_in_that_ended_asks_the_owner_to_sign_in_again_as_the_same_agent() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 0).await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": { "code": "refresh_token_expired", "message": "Expired." }
        })))
        .mount(&server)
        .await;
    let conversation = [Message::from_owner("Hello")];
    let before = asked_in(chatgpt.sign_in().unwrap().link());

    let failed = chatgpt.reply(question(&conversation, &[])).await.unwrap_err();

    let ProviderError::Rejected(reason) = failed else { panic!("{failed:?}") };
    assert!(reason.contains("orbite setup provider"), "{reason}");
    assert!(!chatgpt.is_signed_in());
    // OpenAI knows this Agent already: it signs in again under the name it
    // was given, as the same machine, saying who it was.
    let again = asked_in(chatgpt.sign_in().unwrap().link());
    assert_eq!(again["client_id"], ISSUED);
    assert_eq!(again["ext_agent_host_id"], before["ext_agent_host_id"]);
    assert!(!again.contains_key("agent_name_hint"));
    assert!(again["id_token_hint"].starts_with("ey"));
    assert_eq!(again["login_hint"], "owner@example.org");
}

#[tokio::test]
async fn openai_being_briefly_unable_to_renew_does_not_end_the_sign_in() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 0).await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let conversation = [Message::from_owner("Hello")];

    let failed = chatgpt.reply(question(&conversation, &[])).await.unwrap_err();

    assert!(matches!(failed, ProviderError::Busy { .. }), "{failed:?}");
    assert!(chatgpt.is_signed_in());
}

#[tokio::test]
async fn the_owner_is_told_when_their_plan_has_no_usage_left_and_where_to_manage_it() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 3600).await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({ "error": {
            "code": "subscription_sharing_usage_limit_exceeded", "message": "Limit reached.",
        } })))
        .expect(1)
        .mount(&server)
        .await;
    let conversation = [Message::from_owner("Hello")];

    let failed = chatgpt.reply(question(&conversation, &[])).await.unwrap_err();

    let ProviderError::Busy { reason, retry_after } = failed else { panic!("{failed:?}") };
    assert!(reason.contains("https://chatgpt.com/settings/usage"), "{reason}");
    // Asking again in a few seconds would change nothing.
    assert!(retry_after.is_some_and(|wait| wait.as_secs() >= 3600));
}

#[tokio::test]
async fn a_limit_met_while_answering_is_told_the_same_way() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 3600).await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(stream(&[
            json!({ "type": "response.output_text.delta", "delta": "I was sa" }),
            json!({ "type": "response.failed", "response": { "error": {
                "code": "subscription_sharing_usage_limit_exceeded", "message": "Limit reached.",
            } } }),
        ]))
        .mount(&server)
        .await;
    let conversation = [Message::from_owner("Hello")];

    let failed = chatgpt.reply(question(&conversation, &[])).await.unwrap_err();

    let ProviderError::Busy { reason, .. } = failed else { panic!("{failed:?}") };
    assert!(reason.contains("https://chatgpt.com/settings/usage"), "{reason}");
}

#[tokio::test]
async fn a_plan_that_may_not_be_used_this_way_is_said_plainly() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 3600).await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "error": {
            "code": "subscription_sharing_user_not_eligible", "message": "Not eligible.",
        } })))
        .expect(1)
        .mount(&server)
        .await;
    let conversation = [Message::from_owner("Hello")];

    let failed = chatgpt.reply(question(&conversation, &[])).await.unwrap_err();

    let ProviderError::Rejected(reason) = failed else { panic!("{failed:?}") };
    assert!(reason.contains("Plus or Pro"), "{reason}");
    // Signing in again would not change it: what was kept stays.
    assert!(chatgpt.is_signed_in());
}

#[tokio::test]
async fn an_answer_cut_before_its_end_is_not_taken_for_one() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 3600).await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(stream(&[json!({ "type": "response.output_text.delta", "delta": "Half" })]))
        .mount(&server)
        .await;
    let conversation = [Message::from_owner("Hello")];

    let failed = chatgpt.reply(question(&conversation, &[])).await.unwrap_err();

    assert!(matches!(failed, ProviderError::Busy { .. }), "{failed:?}");
}

#[tokio::test]
async fn asking_before_signing_in_says_how_to_sign_in() {
    let home = tempfile::tempdir().unwrap();
    let chatgpt = ChatGpt::kept_in(home.path(), "gpt-test").at("http://127.0.0.1:9");
    let conversation = [Message::from_owner("Hello")];

    let failed = chatgpt.reply(question(&conversation, &[])).await.unwrap_err();

    let ProviderError::Rejected(reason) = failed else { panic!("{failed:?}") };
    assert!(reason.contains("orbite setup provider"), "{reason}");
}

#[tokio::test]
async fn the_models_offered_are_those_openai_lists_for_the_plan() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 3600).await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer access-first"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [
            { "slug": "gpt-large", "display_name": "GPT Large", "visibility": "list" },
            { "slug": "gpt-inner", "display_name": "Inner", "visibility": "hide" },
            { "slug": "gpt-small", "display_name": "GPT Small", "visibility": "list" },
        ] })))
        .mount(&server)
        .await;

    let offered = chatgpt.models().await;

    assert_eq!(
        offered,
        vec![
            ("gpt-large".to_owned(), "GPT Large".to_owned()),
            ("gpt-small".into(), "GPT Small".into())
        ]
    );
}

mod support;

#[tokio::test]
async fn a_sign_in_that_ended_is_made_again_from_the_conversation() {
    use support::{ScriptedProvider, memory_channel};
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let config = json!({
        "language": "en",
        "provider": { "name": "chatgpt", "model": "gpt-test", "address": server.uri() },
    });
    std::fs::write(home.path().join("config.json"), config.to_string()).unwrap();
    let provider = ScriptedProvider::default();
    let asked = provider.clone();
    let (mut channel, mut owner) = memory_channel();
    let folder = home.path().to_owned();
    tokio::spawn(async move {
        orbite::Agent::new(provider, orbite::Store::in_memory())
            .configured_in(folder)
            .converse(&mut channel)
            .await
    });

    owner.says("/providers connect chatgpt");
    let said = owner.hears().await;
    assert!(said.contains("Continue with ChatGPT"), "{said}");
    let link = said.split_whitespace().find(|word| word.contains("/api/accounts/authorize?"));
    let link = link.expect("a link is sent").to_owned();
    signing_in(&server, &asked_in(&link)["nonce"], 3600).await;

    owner.says(&format!("here it is: {}", ended_on(&link)));
    let said = owner.hears().await;

    assert!(said.contains("You're using your ChatGPT plan (owner@example.org)"), "{said}");
    assert!(said.contains("https://chatgpt.com/settings/usage"), "{said}");
    assert!(ChatGpt::kept_in(home.path(), "gpt-test").is_signed_in());
    assert_eq!(asked.times_asked(), 0, "the address is not shown to a model");
}

#[tokio::test]
async fn removing_the_plan_from_the_providers_signs_out_at_openai_too() {
    let server = MockServer::start().await;
    let (home, chatgpt) = signed_in(&server, 3600).await;
    let config = json!({
        "provider": { "name": "chatgpt", "model": "gpt-test", "address": server.uri() },
        "fallbacks": [{ "name": "ollama", "model": "some-model" }],
    });
    std::fs::write(home.path().join("config.json"), config.to_string()).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/revoke"))
        .and(body_string_contains("token=refresh-first"))
        .and(body_string_contains("token_type_hint=refresh_token"))
        .and(body_string_contains(format!("client_id={ISSUED}")))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    orbite::change_providers(home.path(), &["remove", "chatgpt"]).await.unwrap();

    assert!(!chatgpt.is_signed_in());
    assert_eq!(Secrets::kept_in(home.path()).get("ORBITE_CHATGPT").unwrap(), None);
}

#[tokio::test]
async fn the_models_of_the_plan_are_offered_where_the_owner_changes_the_model() {
    use support::{ScriptedProvider, memory_channel};
    let server = MockServer::start().await;
    let (home, _chatgpt) = signed_in(&server, 3600).await;
    let config = json!({
        "language": "en",
        "provider": { "name": "chatgpt", "model": "gpt-small", "address": server.uri() },
    });
    std::fs::write(home.path().join("config.json"), config.to_string()).unwrap();
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [
            { "slug": "gpt-large", "display_name": "GPT Large", "visibility": "list" },
            { "slug": "gpt-small", "display_name": "GPT Small", "visibility": "list" },
        ] })))
        .mount(&server)
        .await;
    let (mut channel, mut owner) = memory_channel();
    let folder = home.path().to_owned();
    tokio::spawn(async move {
        orbite::Agent::new(ScriptedProvider::default(), orbite::Store::in_memory())
            .configured_in(folder)
            .converse(&mut channel)
            .await
    });

    owner.says("/settings model");
    let offered = owner.hears().await;

    assert!(
        offered.contains("GPT Large") && offered.contains("/settings model gpt-large"),
        "{offered}"
    );
    // The one in force is marked, as for every other setting.
    assert!(offered.contains("● GPT Small"), "{offered}");
}

#[tokio::test]
async fn an_answer_that_comes_whole_rather_than_piece_by_piece_is_read() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 3600).await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(stream(&[
            json!({ "type": "response.output_item.done", "item": { "type": "message",
                "content": [{ "type": "output_text", "text": "All at once." }] } }),
            json!({ "type": "response.completed", "response": { "usage": {} } }),
        ]))
        .mount(&server)
        .await;
    let conversation = [Message::from_owner("Hello")];

    assert_eq!(chatgpt.reply(question(&conversation, &[])).await.unwrap().text, "All at once.");
}

#[tokio::test]
async fn what_the_model_declines_is_said_as_declined_with_its_own_words() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 3600).await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(stream(&[
            json!({ "type": "response.output_item.done", "item": { "type": "message",
                "content": [{ "type": "refusal", "refusal": "I cannot help with that." }] } }),
            json!({ "type": "response.completed", "response": { "usage": {} } }),
        ]))
        .mount(&server)
        .await;
    let conversation = [Message::from_owner("Hello")];

    let failed = chatgpt.reply(question(&conversation, &[])).await.unwrap_err();

    assert_eq!(failed, ProviderError::Declined("I cannot help with that.".into()));
}

#[tokio::test]
async fn openais_own_trouble_lets_another_provider_answer() {
    let server = MockServer::start().await;
    let (_home, chatgpt) = signed_in(&server, 3600).await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(stream(&[json!({ "type": "response.failed", "response": { "error": {
            "code": "server_error", "message": "Something went wrong on our side.",
        } } })]))
        .mount(&server)
        .await;
    let conversation = [Message::from_owner("Hello")];

    // Busy, unlike a failure about the request itself, is tried elsewhere.
    let failed = chatgpt.reply(question(&conversation, &[])).await.unwrap_err();
    assert!(matches!(failed, ProviderError::Busy { .. }), "{failed:?}");
}

#[tokio::test]
async fn an_address_sent_when_no_sign_in_is_under_way_is_neither_kept_nor_shown_to_a_model() {
    use support::{ScriptedProvider, memory_channel};
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Never said."]);
    let asked = provider.clone();
    let (mut channel, mut owner) = memory_channel();
    let folder = home.path().to_owned();
    tokio::spawn(async move {
        orbite::Agent::new(provider, orbite::Store::in_memory())
            .configured_in(folder)
            .converse(&mut channel)
            .await
    });

    owner.says("here it is http://127.0.0.1:1455/auth/callback?code=only-once&state=of-a-link");
    let said = owner.hears().await;

    assert!(said.contains("/providers connect chatgpt"), "{said}");
    assert_eq!(asked.times_asked(), 0);
}

#[tokio::test]
async fn the_model_is_shown_the_pictures_that_come_with_a_notice_such_as_the_computers_screen() {
    let server = MockServer::start().await;
    let (home, chatgpt) = signed_in(&server, 3600).await;
    let sent = Arc::new(Mutex::new(Value::Null));
    let seen = sent.clone();
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(move |request: &wiremock::Request| {
            *seen.lock().unwrap() = serde_json::from_slice(&request.body).unwrap();
            saying("I see it.")
        })
        .mount(&server)
        .await;
    // The smallest PNG there is: its content does not matter here.
    let picture = home.path().join("screen.png");
    std::fs::write(&picture, b"\x89PNG\r\n\x1a\nnot much of a picture").unwrap();
    let conversation = [
        Message::from_owner("Open the site."),
        Message::Notice("The Computer's screen now:".into()),
    ];
    let request = Request {
        instructions: "",
        conversation: &conversation,
        actions: &[],
        images: &[(1, picture)],
        light: false,
    };

    chatgpt.reply(request).await.unwrap();

    let sent = sent.lock().unwrap().clone();
    let notice = &sent["input"][1]["content"];
    assert_eq!(notice[0]["type"], "input_text", "{notice}");
    assert!(notice[0]["text"].as_str().unwrap().contains("The Computer's screen now:"));
    assert_eq!(notice[1]["type"], "input_image", "{notice}");
    assert!(notice[1]["image_url"].as_str().unwrap().starts_with("data:image/png;base64,"));
}
