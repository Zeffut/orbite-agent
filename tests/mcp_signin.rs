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

#[tokio::test]
async fn a_server_that_takes_no_newcomer_is_signed_in_to_with_a_client_the_owner_brings() {
    let server = MockServer::start().await;
    let here = server.uri();
    let secret = ["client", "secret", "for", "the", "test"].join("-");
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
    // The code is exchanged only by the client the Owner registered, with its secret.
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("client_id=the-owners-own-client"))
        .and(body_string_contains(format!("client_secret={secret}").as_str()))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({ "access_token": "access-for-the-test", "expires_in": 3600 }),
            ),
        )
        .mount(&server)
        .await;
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("FAR_CLIENT_SECRET", &secret).unwrap();
    std::fs::write(
        home.path().join("mcp.json"),
        json!({ "mcpServers": { "far": {
            "url": format!("{here}/mcp"),
            "client": { "id": "the-owners-own-client", "secret": "${FAR_CLIENT_SECRET}" },
        } } })
        .to_string(),
    )
    .unwrap();
    let mut owner = agent_in(home.path(), &ScriptedProvider::default());

    owner.says("/mcp connect far");
    let said = owner.hears().await;
    let link = carried(&said);
    assert_eq!(link["client_id"], "the-owners-own-client");
    assert!(!said.contains(&secret), "the client's secret was shown");

    owner.says(&format!("{}?code=the-code&state={}", link["redirect_uri"], link["state"]));
    owner.hears().await;

    // Signed in: the access is kept (the server offering nothing here is another matter).
    let kept = Secrets::kept_in(home.path())
        .get("ORBITE_MCP_FAR")
        .unwrap()
        .expect("the sign-in was not finished");
    assert!(kept.contains("access-for-the-test") && !kept.contains(&secret));
}

// --- Every connection in one place ---

/// An Agent of the installation in `home`, which keeps what it is told across restarts.
fn installed_agent(
    home: &std::path::Path,
    provider: &ScriptedProvider,
) -> (Owner, tokio::task::JoinHandle<()>) {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let home = home.to_owned();
    let running = tokio::spawn(async move {
        Agent::new(provider, Store::open(home.join("orbite.db")).unwrap())
            .configured_in(&home)
            .keeping_secrets_in(&home)
            .connecting_servers_in(&home)
            .restarted_by_its_service()
            .converse(&mut channel)
            .await
    });
    (owner, running)
}

#[tokio::test]
async fn the_owner_sees_every_connection_in_one_place_with_what_to_do_about_each() {
    let server = service(Arc::new(Mutex::new(String::new()))).await;
    let home = home_with(&server).await;
    let config = json!({
        "language": "en",
        "provider": { "name": "chatgpt", "model": "gpt-of-the-plan" },
        "fallbacks": [{ "name": "groq", "model": "m-1", "key": "ORBITE_KEY_GROQ" },
                      { "name": "ollama", "model": "m-2" }],
    });
    std::fs::write(home.path().join("config.json"), config.to_string()).unwrap();
    Secrets::kept_in(home.path()).set("ORBITE_KEY_GROQ", &format!("key-{}", "of-groq")).unwrap();
    let provider = ScriptedProvider::default();
    let (mut owner, _running) = installed_agent(home.path(), &provider);

    owner.says("/connections");
    let shown = owner.hears().await;

    // The AIs, in the order they are asked, each with its state.
    let at = |text: &str| shown.find(text).unwrap_or_else(|| panic!("{text} in:\n{shown}"));
    assert!(at("chatgpt") < at("groq") && at("groq") < at("ollama"), "{shown}");
    assert!(shown.contains("/providers connect chatgpt"), "not signed in, and how to: {shown}");
    assert!(shown[at("groq")..at("ollama")].contains("🟢"), "its key is kept: {shown}");
    // The connectors, and the one to sign in to.
    assert!(shown.contains("far") && shown.contains("/mcp connect far"), "{shown}");
    // And how to add one, with nothing to edit by hand.
    assert!(shown.contains("I can connect, just ask") && shown.contains("Supabase"), "{shown}");
    // The Owner is never sent to a command.
    assert!(!shown.contains("orbite providers"), "{shown}");
    assert_eq!(provider.times_asked(), 0, "none of this is sent to a model");
}

#[tokio::test]
async fn a_connector_is_added_by_its_address_and_signed_in_to_once_orbite_is_back() {
    let challenge = Arc::new(Mutex::new(String::new()));
    let server = service(challenge.clone()).await;
    let home = tempfile::tempdir().unwrap();
    // Another one is there already, and stays as it was written.
    std::fs::write(
        home.path().join("mcp.json"),
        json!({ "mcpServers": { "notes": { "command": "true", "args": ["--quiet"] } } })
            .to_string(),
    )
    .unwrap();
    let provider = ScriptedProvider::default();
    let (mut owner, running) = installed_agent(home.path(), &provider);

    owner.says(&format!("/connect far {}/mcp", server.uri()));
    let said = owner.hears().await;
    assert!(said.contains("far") && said.to_lowercase().contains("back in a moment"), "{said}");
    tokio::time::timeout(std::time::Duration::from_secs(2), running)
        .await
        .expect("the Agent ends so that its service starts it again")
        .unwrap();
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(home.path().join("mcp.json")).unwrap())
            .unwrap();
    assert_eq!(written["mcpServers"]["far"]["url"], format!("{}/mcp", server.uri()));
    assert_eq!(written["mcpServers"]["notes"]["args"], json!(["--quiet"]));

    // Back, it goes on by itself: this connector wants signing in, here is the link.
    let (mut owner, _running) = installed_agent(home.path(), &provider);
    let said = owner.hears().await;
    assert!(said.contains("far"), "{said}");
    let asked = carried(&said);
    assert_eq!(asked["client_id"], "client-made-for-orbite");
    assert_eq!(provider.times_asked(), 0);
}

#[tokio::test]
async fn what_cannot_be_a_connector_is_refused_and_nothing_is_written() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("mcp.json"),
        json!({ "mcpServers": { "far": { "url": "https://far.example/mcp" } } }).to_string(),
    )
    .unwrap();
    let before = std::fs::read_to_string(home.path().join("mcp.json")).unwrap();
    let provider = ScriptedProvider::default();
    let (mut owner, _running) = installed_agent(home.path(), &provider);

    for (asked, why) in [
        ("/connect docs not-an-address", "address"),
        ("/connect docs ftp://docs.example/mcp", "address"),
        ("/connect far https://other.example/mcp", "already"),
        ("/connect computer https://other.example/mcp", "name"),
        ("/connect docs", "its name and its address"),
    ] {
        owner.says(asked);
        let said = owner.hears().await;
        assert!(said.contains(why), "{asked}: {said}");
    }
    assert_eq!(std::fs::read_to_string(home.path().join("mcp.json")).unwrap(), before);
}

#[tokio::test]
async fn a_connector_is_signed_out_of_or_removed_from_the_conversation() {
    let server = service(Arc::new(Mutex::new(String::new()))).await;
    let home = home_with(&server).await;
    let secrets = Secrets::kept_in(home.path());
    secrets
        .set("ORBITE_MCP_FAR", r#"{"access":"a","client":"c","exchange":"e","resource":"r"}"#)
        .unwrap();
    let provider = ScriptedProvider::default();
    let (mut owner, _running) = installed_agent(home.path(), &provider);

    owner.says("/connections disconnect far");
    let said = owner.hears().await;
    assert!(said.contains("far"), "{said}");
    assert_eq!(secrets.get("ORBITE_MCP_FAR").unwrap(), None);
    assert!(std::fs::read_to_string(home.path().join("mcp.json")).unwrap().contains("far"));

    owner.says("/connections remove nothing-of-that-name");
    assert!(owner.hears().await.contains("far"), "the ones there are named");

    owner.says("/connections remove far");
    let said = owner.hears().await;
    assert!(said.to_lowercase().contains("removed"), "{said}");
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(home.path().join("mcp.json")).unwrap())
            .unwrap();
    assert!(written["mcpServers"].get("far").is_none(), "{written}");
}

/// An MCP server that lets in only who carries `access`, with one tool, and
/// refuses everybody else.
async fn server_taking_only(access: &str) -> MockServer {
    let server = MockServer::start().await;
    let answering = |result: Value| {
        move |request: &wiremock::Request| {
            let asked: Value = serde_json::from_slice(&request.body).unwrap_or_default();
            ResponseTemplate::new(200)
                .set_body_json(json!({ "jsonrpc": "2.0", "id": asked["id"], "result": result }))
        }
    };
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
            .and(header("authorization", format!("Bearer {access}").as_str()))
            .and(header("mcp-method", asked))
            .respond_with(answering(result))
            .mount(&server)
            .await;
    }
    Mock::given(path("/mcp")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
    server
}

/// A home whose Owner signed in to `server` a while ago, and was given an
/// access said to last a good while yet.
async fn home_signed_in_to(server: &MockServer) -> tempfile::TempDir {
    let home = home_with(server).await;
    let here = server.uri();
    let later =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
            + 3600;
    let kept = json!({
        "access": "old-access", "refresh": "the-refresh", "until": later, "client": "client-made-for-orbite",
        "exchange": format!("{here}/token"), "resource": format!("{here}/mcp"),
    });
    Secrets::kept_in(home.path()).set("ORBITE_MCP_FAR", &kept.to_string()).unwrap();
    home
}

#[tokio::test]
async fn an_access_refused_before_its_time_is_renewed_and_the_tool_still_used() {
    let server = server_taking_only("renewed-access").await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=the-refresh"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "access_token": "renewed-access", "expires_in": 3600 })),
        )
        .mount(&server)
        .await;
    let home = home_signed_in_to(&server).await;
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
    assert!(
        !told.iter().any(|message| matches!(message, Message::Notice(_))),
        "nothing was to be told: {told:?}"
    );
}

#[tokio::test]
async fn a_sign_in_that_ended_is_told_to_the_owner_once_with_how_to_sign_in_again() {
    // Nothing is let in any more, and what renews the access refuses too.
    let server = server_taking_only("an-access-nobody-has").await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" })))
        .mount(&server)
        .await;
    let home = home_signed_in_to(&server).await;
    let provider = ScriptedProvider::default();
    provider.then(Ok("Your sign-in to far has ended.".into())).then(Ok("Nothing new.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Good morning.");
    assert_eq!(owner.hears().await, "Your sign-in to far has ended.");
    let notices = |told: &[Message]| -> Vec<String> {
        told.iter()
            .filter_map(|message| match message {
                Message::Notice(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    };
    let first = notices(&provider.last_asked());
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].contains("far") && first[0].contains("/mcp connect far"), "{}", first[0]);

    // Said once: the next message brings no second telling.
    owner.says("Anything else?");
    assert_eq!(owner.hears().await, "Nothing new.");
    assert_eq!(notices(&provider.last_asked()).len(), 1);

    // And where connections are listed, it is shown as to be signed in to.
    owner.says("/connections");
    let listed = owner.hears().await;
    assert!(listed.contains("far") && listed.contains("/mcp connect far"), "{listed}");
}

#[tokio::test]
async fn a_connector_turned_off_is_kept_with_its_sign_in_and_left_alone_until_turned_on() {
    let server = server_taking_only("old-access").await;
    let home = home_signed_in_to(&server).await;
    let list = || -> Value {
        serde_json::from_str(&std::fs::read_to_string(home.path().join("mcp.json")).unwrap())
            .unwrap()
    };
    let provider = ScriptedProvider::default();
    let (mut owner, running) = installed_agent(home.path(), &provider);

    owner.says("/connections off far");
    let said = owner.hears().await;
    assert!(said.contains("far") && said.contains("turned off"), "{said}");
    assert_eq!(list()["mcpServers"]["far"]["disabled"], json!(true));
    assert!(
        Secrets::kept_in(home.path()).get("ORBITE_MCP_FAR").unwrap().is_some(),
        "its sign-in was not kept"
    );
    // Orbite starts again to take it, as its service has it do.
    let _ = running.await;

    // Back, its tools are not offered and the server is not even reached.
    let before = server.received_requests().await.unwrap().len();
    let provider = ScriptedProvider::default();
    provider.then(Ok("Hello.".into()));
    let (mut owner, running) = installed_agent(home.path(), &provider);
    owner.says("Good morning.");
    assert_eq!(owner.hears().await, "Hello.");
    assert!(!provider.last_offered().iter().any(|action| action.contains("far")));
    assert_eq!(server.received_requests().await.unwrap().len(), before);

    // It is still listed, as off, with the way to turn it on.
    owner.says("/connections");
    let listed = owner.hears().await;
    assert!(listed.contains("far") && listed.contains("/connections on far"), "{listed}");

    owner.says("/connections on far");
    let said = owner.hears().await;
    assert!(said.contains("turned on"), "{said}");
    assert!(list()["mcpServers"]["far"].get("disabled").is_none(), "{}", list());
    let _ = running.await;

    // Back once more, its tools are the Agent's again, with no new sign-in.
    let provider = ScriptedProvider::default();
    provider.then(Ok("Hello again.".into()));
    let (mut owner, _running) = installed_agent(home.path(), &provider);
    owner.says("And now?");
    assert_eq!(owner.hears().await, "Hello again.");
    assert!(provider.last_offered().iter().any(|action| action == "far_lookup"));
}

#[tokio::test]
async fn asked_in_plain_words_the_agent_adds_a_connector_once_its_owner_agrees() {
    let server = service(Arc::new(Mutex::new(String::new()))).await;
    let home = tempfile::tempdir().unwrap();
    let address = format!("{}/mcp", server.uri()).replace("http://", "https://");
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": format!("/connect far {address}") }))])
        .then(Ok("On it.".into()));
    let (mut owner, running) = installed_agent(home.path(), &provider);

    owner.says("Add the far connector for me.");
    // A new source of tools is the Owner's to agree to.
    let asked = owner.is_asked().await;
    assert!(asked.summary.contains("Add the connector far"), "{}", asked.summary);
    assert!(asked.detail.contains(&address), "{}", asked.detail);
    owner.allows();
    assert_eq!(owner.hears().await, "On it.");
    // Orbite then does it and says so itself, and starts again to take it.
    let said = owner.hears().await;
    assert!(said.contains("far"), "{said}");
    let _ = running.await;
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(home.path().join("mcp.json")).unwrap())
            .unwrap();
    assert_eq!(written["mcpServers"]["far"]["url"], json!(address));
}

#[tokio::test]
async fn what_the_agent_does_for_its_owner_is_all_done_though_one_of_them_restarts_orbite() {
    let server = server_taking_only("old-access").await;
    let home = home_signed_in_to(&server).await;
    let provider = ScriptedProvider::default();
    // Turning the connector off restarts Orbite: asked first, done last.
    provider
        .then_acts([
            ("command", json!({ "command": "/connections off far" })),
            ("command", json!({ "command": "/usage" })),
        ])
        .then(Ok("Both are on their way.".into()));
    let (mut owner, running) = installed_agent(home.path(), &provider);

    owner.says("Turn far off and tell me what I used today.");
    owner.is_asked().await;
    owner.allows();
    assert_eq!(owner.hears().await, "Both are on their way.");
    let first = owner.hears().await;
    assert!(first.contains("Today") || first.contains("used"), "the use came first: {first}");
    let second = owner.hears().await;
    assert!(second.contains("far") && second.contains("turned off"), "{second}");
    let _ = running.await;
}

#[tokio::test]
async fn a_service_is_connected_with_a_touch_and_the_sign_in_ends_on_the_agents_own_page() {
    let challenge = Arc::new(Mutex::new(String::new()));
    let server = service(challenge.clone()).await;
    let home = tempfile::tempdir().unwrap();
    // An Agent whose page is reached encrypted, at a name of its own.
    std::fs::write(
        home.path().join("config.json"),
        json!({ "interface": "127.0.0.1:8377", "interface_reached_at": "https://agent.example.ts.net:8377" })
            .to_string(),
    )
    .unwrap();
    let address: &'static str = Box::leak(format!("{}/mcp", server.uri()).into_boxed_str());
    let offered = [orbite::Connector {
        name: "notes",
        label: "Notes",
        address,
        english: "Find your notes",
        french: "Retrouver tes notes",
    }];
    let interface = orbite::Interface::serving_with(
        home.path(),
        "127.0.0.1:0".parse().unwrap(),
        "en",
        &offered,
    )
    .await
    .unwrap();
    let at = format!("http://{}", interface.address());
    let http = reqwest::Client::new();
    // A device let in, as the page or an app is.
    let code = orbite::pairing_code(home.path()).unwrap();
    let paired = http
        .post(format!("{at}/api/pair"))
        .header("x-orbite", "1")
        .json(&json!({ "code": code, "name": "phone" }))
        .send()
        .await
        .unwrap();
    let kept =
        paired.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_owned();
    let listed = || async {
        let all: Value = http
            .get(format!("{at}/api/connectors"))
            .header("cookie", &kept)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        all
    };

    // Offered, not connected yet, and connectable with a touch.
    let before = listed().await;
    assert_eq!(before["with_a_touch"], true);
    assert_eq!(before["connectors"][0]["label"], "Notes");
    assert_eq!(before["connectors"][0]["connected"], false);
    // A stranger is told nothing, and begins nothing.
    assert_eq!(http.get(format!("{at}/api/connectors")).send().await.unwrap().status(), 401);

    // Touched: the link to sign in with, which ends on the Agent's own page.
    let begun: Value = http
        .post(format!("{at}/api/connectors"))
        .header("x-orbite", "1")
        .header("cookie", &kept)
        .json(&json!({ "name": "notes" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let link: std::collections::HashMap<String, String> =
        url::Url::parse(begun["link"].as_str().expect("no link"))
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
    assert_eq!(link["redirect_uri"], "https://agent.example.ts.net:8377/signed-in");
    *challenge.lock().unwrap() = link["code_challenge"].clone();

    // Someone who is not the answer to that link changes nothing.
    let stranger =
        http.get(format!("{at}/signed-in?code=guessed&state=guessed")).send().await.unwrap();
    assert!(stranger.text().await.unwrap().contains("did not go through"));
    assert_eq!(listed().await["connectors"][0]["connected"], false);

    // The sign-in under way is still there for the Owner's own browser,
    // which comes back from the service.
    let back = http
        .get(format!("{at}/signed-in?code=the-code&state={}&iss={}", link["state"], server.uri()))
        .send()
        .await
        .unwrap();
    let page = back.text().await.unwrap();
    assert!(page.contains("Notes is connected."), "{page}");
    assert!(!page.contains("the-code"), "{page}");
    assert_eq!(listed().await["connectors"][0]["connected"], true);
    // And it is among the services the Agent knows from its next start.
    assert!(std::fs::read_to_string(home.path().join("mcp.json")).unwrap().contains("notes"));
}

#[tokio::test]
async fn asked_for_in_the_conversation_a_sign_in_opens_on_the_owners_device_with_nothing_to_copy() {
    use orbite::Channel;
    let challenge = Arc::new(Mutex::new(String::new()));
    let server = service(challenge.clone()).await;
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.json"),
        json!({ "interface": "127.0.0.1:8377", "interface_reached_at": "https://agent.example.ts.net:8377" })
            .to_string(),
    )
    .unwrap();
    let address: &'static str = Box::leak(format!("{}/mcp", server.uri()).into_boxed_str());
    let offered = [orbite::Connector {
        name: "notes",
        label: "Notes",
        address,
        english: "Find your notes",
        french: "Retrouver tes notes",
    }];
    let mut interface = orbite::Interface::serving_with(
        home.path(),
        "127.0.0.1:0".parse().unwrap(),
        "en",
        &offered,
    )
    .await
    .unwrap();
    let at = format!("http://{}", interface.address());
    let http = reqwest::Client::new();
    let code = orbite::pairing_code(home.path()).unwrap();
    let paired = http
        .post(format!("{at}/api/pair"))
        .header("x-orbite", "1")
        .json(&json!({ "code": code, "name": "phone" }))
        .send()
        .await
        .unwrap();
    let kept =
        paired.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_owned();
    // The device is listening, as an app left open is.
    let mut listening =
        http.get(format!("{at}/api/events")).header("cookie", &kept).send().await.unwrap();

    // What the Agent does when its Owner says: connect yourself to my notes.
    assert_eq!(interface.sign_in("notes").await, Some(Ok("Notes".to_owned())));
    // A service nobody named is not begun.
    assert!(matches!(interface.sign_in("unheard-of").await, Some(Err(_))));

    // The device is told where to sign in, and opens it.
    let mut told = String::new();
    while !told.contains("\"sign-in\"") {
        let more = tokio::time::timeout(std::time::Duration::from_secs(5), listening.chunk())
            .await
            .expect("the device was told nothing")
            .unwrap()
            .expect("the stream ended");
        told.push_str(&String::from_utf8_lossy(&more));
    }
    let event: Value = told
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|body| serde_json::from_str::<Value>(body.trim()).ok())
        .find(|event| event["kind"] == "sign-in")
        .unwrap();
    assert_eq!(event["label"], "Notes");
    let link: std::collections::HashMap<String, String> =
        url::Url::parse(event["link"].as_str().unwrap())
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
    // It ends on the Agent's own page: nothing is copied back by hand.
    assert_eq!(link["redirect_uri"], "https://agent.example.ts.net:8377/signed-in");
    *challenge.lock().unwrap() = link["code_challenge"].clone();

    let back = http
        .get(format!("{at}/signed-in?code=the-code&state={}&iss={}", link["state"], server.uri()))
        .send()
        .await
        .unwrap();
    assert!(back.text().await.unwrap().contains("Notes is connected."));
    // The Agent is told, for when it is back: it says so and carries on.
    let noticed: Vec<String> = std::fs::read_dir(home.path().join("notices"))
        .unwrap()
        .flatten()
        .filter_map(|file| std::fs::read_to_string(file.path()).ok())
        .collect();
    assert!(
        noticed
            .iter()
            .any(|notice| notice.contains("signed in to Notes") && notice.contains("carry on")),
        "{noticed:?}"
    );
}
