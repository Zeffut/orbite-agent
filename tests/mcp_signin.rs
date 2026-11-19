//! What an Owner goes through to connect an MCP server that needs signing
//! in: a link to follow, and the address their browser ends on to send back.

mod support;

use std::sync::{Arc, Mutex};

use base64::Engine;
use orbite::{Agent, Message, Secrets, Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::{Owner, ScriptedProvider, memory_channel};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A service that is both an MCP server asking for a signed-in caller and
/// the place where one signs in. `challenge` is what the Owner's browser
/// would have carried there.
async fn service(challenge: Arc<Mutex<String>>) -> MockServer {
    let server = MockServer::start().await;
    let here = server.uri();
    let answering = |result: Value| {
        move |request: &wiremock::Request| {
            let asked: Value = serde_json::from_slice(&request.body).unwrap_or_default();
            ResponseTemplate::new(200)
                .set_body_json(json!({ "jsonrpc": "2.0", "id": asked["id"], "result": result }))
        }
    };
    // Signed in: the server answers.
    for (asked, result) in [
        (
            "server/discover",
            json!({ "resultType": "complete", "supportedVersions": ["2026-07-28"] }),
        ),
        (
            "tools/list",
            json!({ "resultType": "complete", "tools": [{
            "name": "lookup", "description": "Looks a word up.",
            "inputSchema": { "type": "object", "properties": { "word": { "type": "string" } } },
            "annotations": { "readOnlyHint": true },
        }] }),
        ),
        (
            "tools/call",
            json!({ "resultType": "complete", "content": [{ "type": "text", "text": "hello" }] }),
        ),
    ] {
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .and(header("authorization", "Bearer access-for-the-test"))
            .and(header("mcp-method", asked))
            .respond_with(answering(result))
            .mount(&server)
            .await;
    }
    // Not signed in: it says where to learn how.
    Mock::given(path("/mcp"))
        .respond_with(
            ResponseTemplate::new(401).insert_header(
                "www-authenticate",
                format!(
                    "Bearer resource_metadata=\"{here}/.well-known/oauth-protected-resource/mcp\""
                )
                .as_str(),
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{here}/mcp"), "authorization_servers": [here], "scopes_supported": ["read"],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": here,
            "authorization_endpoint": format!("{here}/authorize"),
            "token_endpoint": format!("{here}/token"),
            "registration_endpoint": format!("{here}/register"),
            "code_challenge_methods_supported": ["S256"],
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/register"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({ "client_id": "client-made-for-orbite" })),
        )
        .mount(&server)
        .await;
    // The code is exchanged only by whoever holds what the challenge was made from.
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=the-code"))
        .respond_with(move |request: &wiremock::Request| {
            let sent: std::collections::HashMap<String, String> =
                url::form_urlencoded::parse(&request.body).into_owned().collect();
            let made = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(
                sent.get("code_verifier").cloned().unwrap_or_default().as_bytes(),
            ));
            if made == *challenge.lock().unwrap()
                && sent.get("client_id").is_some_and(|id| id == "client-made-for-orbite")
            {
                ResponseTemplate::new(200).set_body_json(json!({
                    "access_token": "access-for-the-test", "token_type": "Bearer",
                    "refresh_token": "refresh-for-the-test", "expires_in": 3600,
                }))
            } else {
                ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" }))
            }
        })
        .mount(&server)
        .await;
    server
}

fn agent_in(home: &std::path::Path, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let home = home.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .keeping_secrets_in(&home)
            .connecting_servers_in(&home)
            .converse(&mut channel)
            .await
    });
    owner
}

/// What the link the Owner is sent carries, by name.
fn carried(said: &str) -> std::collections::HashMap<String, String> {
    let link = said
        .split_whitespace()
        .find(|word| word.contains("/authorize?"))
        .expect("no link to follow");
    let link = link.trim_matches(|c| c == '<' || c == '>' || c == ')' || c == '(');
    url::Url::parse(link).unwrap().query_pairs().into_owned().collect()
}

async fn home_with(server: &MockServer) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("mcp.json"),
        json!({ "mcpServers": { "far": { "url": format!("{}/mcp", server.uri()) } } }).to_string(),
    )
    .unwrap();
    home
}

#[tokio::test]
async fn a_server_is_connected_by_following_a_link_and_sending_back_where_it_ends() {
    let challenge = Arc::new(Mutex::new(String::new()));
    let server = service(challenge.clone()).await;
    let home = home_with(&server).await;
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("far_lookup", json!({ "word": "salut" }))])
        .then(Ok("It means hello.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("/mcp connect far");
    let said = owner.hears().await;
    let link = carried(&said);
    assert_eq!(link["response_type"], "code");
    assert_eq!(link["client_id"], "client-made-for-orbite");
    assert_eq!(link["code_challenge_method"], "S256");
    assert_eq!(link["resource"], format!("{}/mcp", server.uri()));
    assert!(link["redirect_uri"].starts_with("http://127.0.0.1:"), "{link:?}");
    assert!(said.contains("127.0.0.1"), "the Owner is not told what to send back: {said}");
    *challenge.lock().unwrap() = link["code_challenge"].clone();

    // The Owner signs in; their browser ends on an address that does not load. They send it.
    owner.says(&format!(
        "{}?code=the-code&state={}&iss={}",
        link["redirect_uri"],
        link["state"],
        server.uri()
    ));
    let connected = owner.hears().await;
    assert!(connected.contains("far") && connected.contains('1'), "{connected}");
    assert_eq!(provider.times_asked(), 0, "what the Owner sent back went to the model");

    // Its tools are the Agent's from then on.
    owner.says("What does salut mean?");
    assert_eq!(owner.hears().await, "It means hello.");
    let told = provider.last_asked();
    assert!(
        told.iter()
            .any(|message| matches!(message, Message::Outcome { text, .. } if text == "hello")),
        "{told:?}"
    );
    // Nothing of the sign-in is in the Conversation, and what was obtained is sealed.
    let leaked = told.iter().any(|message| {
        format!("{message:?}").contains("the-code")
            || format!("{message:?}").contains("access-for-the-test")
    });
    assert!(!leaked, "{told:?}");
    let kept =
        Secrets::kept_in(home.path()).get("ORBITE_MCP_FAR").unwrap().expect("nothing was kept");
    assert!(kept.contains("access-for-the-test") && kept.contains("refresh-for-the-test"));
    let on_disk = std::fs::read_to_string(home.path().join("secrets.sealed")).unwrap();
    assert!(!on_disk.contains("access-for-the-test"));
}

#[tokio::test]
async fn an_address_that_is_not_the_answer_to_the_link_sent_is_refused() {
    let challenge = Arc::new(Mutex::new(String::new()));
    let server = service(challenge.clone()).await;
    let home = home_with(&server).await;
    let mut owner = agent_in(home.path(), &ScriptedProvider::default());

    owner.says("/mcp connect far");
    let link = carried(&owner.hears().await);
    *challenge.lock().unwrap() = link["code_challenge"].clone();

    // An address made up by someone else: another state.
    owner.says(&format!("{}?code=the-code&state=not-the-one-sent", link["redirect_uri"]));
    let refused = owner.hears().await;
    assert!(refused.contains("/mcp connect far"), "{refused}");
    assert_eq!(Secrets::kept_in(home.path()).get("ORBITE_MCP_FAR").unwrap(), None);
    let exchanged = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|request| request.url.path() == "/token");
    assert!(!exchanged, "a code was exchanged for an address nobody asked for");
}

#[tokio::test]
async fn a_server_that_takes_no_newcomer_is_explained() {
    // It asks for signing in, but offers no way for a program to introduce itself.
    let server = MockServer::start().await;
    let here = server.uri();
    Mock::given(path("/mcp")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "resource": format!("{here}/mcp"), "authorization_servers": [here] }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": here, "authorization_endpoint": format!("{here}/authorize"), "token_endpoint": format!("{here}/token"),
            "code_challenge_methods_supported": ["S256"],
        })))
        .mount(&server)
        .await;
    let home = home_with(&server).await;
    let mut owner = agent_in(home.path(), &ScriptedProvider::default());

    owner.says("/mcp connect far");
    let said = owner.hears().await;

    assert!(said.contains("far") && !said.contains("/authorize?"), "{said}");
    assert!(
        said.to_lowercase().contains("key") || said.to_lowercase().contains("client"),
        "{said}"
    );
}

#[tokio::test]
async fn what_a_sign_in_obtained_is_renewed_when_it_runs_out() {
    let server = MockServer::start().await;
    let here = server.uri();
    let answering = |result: Value| {
        move |request: &wiremock::Request| {
            let asked: Value = serde_json::from_slice(&request.body).unwrap_or_default();
            ResponseTemplate::new(200)
                .set_body_json(json!({ "jsonrpc": "2.0", "id": asked["id"], "result": result }))
        }
    };
    // Only the renewed access is taken.
    for (asked, result) in [
        (
            "server/discover",
            json!({ "resultType": "complete", "supportedVersions": ["2026-07-28"] }),
        ),
        (
            "tools/list",
            json!({ "resultType": "complete", "tools": [{
            "name": "lookup", "description": "Looks a word up.", "inputSchema": { "type": "object" },
            "annotations": { "readOnlyHint": true },
        }] }),
        ),
        (
            "tools/call",
            json!({ "resultType": "complete", "content": [{ "type": "text", "text": "hello" }] }),
        ),
    ] {
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .and(header("authorization", "Bearer renewed-access"))
            .and(header("mcp-method", asked))
            .respond_with(answering(result))
            .mount(&server)
            .await;
    }
    Mock::given(path("/mcp")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=the-refresh"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "access_token": "renewed-access", "expires_in": 3600 })),
        )
        .mount(&server)
        .await;
    let home = home_with(&server).await;
    // Signed in some time ago: what was obtained then has run out.
    let kept = json!({
        "access": "old-access", "refresh": "the-refresh", "until": 1, "client": "client-made-for-orbite",
        "exchange": format!("{here}/token"), "resource": format!("{here}/mcp"),
    });
    Secrets::kept_in(home.path()).set("ORBITE_MCP_FAR", &kept.to_string()).unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("far_lookup", json!({ "word": "salut" }))]).then(Ok("Hello.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("What does salut mean?");
    assert_eq!(owner.hears().await, "Hello.");

    let told = provider.last_asked();
    assert!(
        told.iter()
            .any(|message| matches!(message, Message::Outcome { text, .. } if text == "hello")),
        "{told:?}"
    );
    let kept = Secrets::kept_in(home.path()).get("ORBITE_MCP_FAR").unwrap().unwrap();
    assert!(
        kept.contains("renewed-access") && kept.contains("the-refresh"),
        "the renewed access was not kept"
    );
}
