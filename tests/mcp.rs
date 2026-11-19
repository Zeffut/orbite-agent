//! What an Owner observes of MCP servers: their tools become Actions of the
//! Agent, behind the same Approval gate as every other Action.

mod support;

use std::fs;
use std::path::Path;

use orbite::{Agent, Message, Secrets, Store};
use serde_json::{Value, json};
use support::{Owner, ScriptedProvider, memory_channel};
use wiremock::matchers::{body_partial_json, header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SERVER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/fake_mcp.py");

/// A home whose mcp.json holds `servers`.
fn home_with(servers: Value) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("mcp.json"), json!({ "mcpServers": servers }).to_string()).unwrap();
    home
}

/// A home with the notes server, run as a local command.
fn home_with_notes() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let calls = home.path().join("calls.log");
    let servers = json!({ "notes": { "command": "python3", "args": [SERVER, calls] } });
    fs::write(home.path().join("mcp.json"), json!({ "mcpServers": servers }).to_string()).unwrap();
    home
}

fn calls(home: &Path) -> Vec<Value> {
    fs::read_to_string(home.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn agent_in(home: &Path, provider: &ScriptedProvider) -> Owner {
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

fn outcome(provider: &ScriptedProvider) -> String {
    match provider.last_asked().pop() {
        Some(Message::Outcome { text, .. }) => text,
        other => panic!("the model was not given an Outcome but {other:?}"),
    }
}

#[tokio::test]
async fn the_tools_of_a_local_server_are_actions_of_the_agent() {
    let home = home_with_notes();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("notes_read_note", json!({ "name": "shopping" }))])
        .then(Ok("Read.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Read my shopping note.");
    assert_eq!(owner.hears().await, "Read.");

    assert!(owner.was_asked_nothing(), "a tool that only reads asked for an Approval");
    assert_eq!(outcome(&provider), "read-note shopping: token none\nsecond part");
    assert_eq!(calls(home.path())[0]["name"], "read-note");
    let offered = provider.last_offered();
    assert!(
        offered.contains(&"notes_read_note".to_owned())
            && offered.contains(&"notes_delete_note".to_owned()),
        "{offered:?}"
    );
}

#[tokio::test]
async fn a_tool_that_changes_something_asks_the_owner_first() {
    let home = home_with_notes();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("notes_delete_note", json!({ "name": "shopping" }))])
        .then_acts([("notes_delete_note", json!({ "name": "old" }))])
        .then(Ok("Done.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Delete my shopping note, and the old one.");
    let asked = owner.is_asked().await;
    assert!(
        asked.summary.contains("Deletes the note of that name") && asked.summary.contains("notes"),
        "{asked:?}"
    );
    assert!(asked.detail.contains("shopping") && asked.irreversible, "{asked:?}");
    owner.denies();
    owner.is_asked().await;
    owner.allows();
    owner.hears().await;

    let carried_out = calls(home.path());
    assert_eq!(carried_out.len(), 1, "a denied tool was called: {carried_out:?}");
    assert_eq!(carried_out[0]["arguments"]["name"], "old");
}

#[tokio::test]
async fn a_tool_that_fails_tells_the_model_so() {
    let home = home_with_notes();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("notes_read_note", json!({ "name": "missing" }))])
        .then(Ok("None.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Read the missing note.");
    owner.hears().await;

    let told = outcome(&provider);
    assert!(told.contains("failed") && told.contains("There is no such note."), "{told}");
}

#[tokio::test]
async fn a_server_is_given_secrets_by_name_without_them_being_written_in_its_settings() {
    let home = tempfile::tempdir().unwrap();
    let value = ["notes", "key", "for", "the", "test"].join("-");
    Secrets::kept_in(home.path()).set("NOTES_TOKEN", &value).unwrap();
    let servers = json!({ "notes": {
        "command": "python3",
        "args": [SERVER, home.path().join("calls.log")],
        "env": { "NOTES_TOKEN": "${NOTES_TOKEN}" },
    } });
    fs::write(home.path().join("mcp.json"), json!({ "mcpServers": servers }).to_string()).unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("notes_read_note", json!({ "name": "shopping" }))])
        .then(Ok("Read.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Read my shopping note.");
    owner.hears().await;

    // The server had the secret; the model is not shown it.
    let told = outcome(&provider);
    assert!(told.starts_with("read-note shopping: token ") && !told.contains("none"), "{told}");
    assert!(!told.contains(&value), "the secret was shown to the model: {told}");
}

#[tokio::test]
async fn a_server_that_does_not_start_is_reported_and_the_agent_carries_on() {
    let home = home_with(json!({
        "broken": { "command": "orbite-test-no-such-command" },
        "notes": { "command": "python3", "args": [SERVER, "/dev/null"] },
    }));
    let provider = ScriptedProvider::answering(["Hi."]);
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Hi");
    assert_eq!(owner.hears().await, "Hi.");
    assert!(provider.last_offered().contains(&"notes_read_note".to_owned()));

    owner.says("/mcp");
    let listed = owner.hears().await;
    assert!(listed.contains("notes") && listed.contains("2"), "{listed}");
    assert!(
        listed.contains("broken") && listed.contains("orbite-test-no-such-command"),
        "{listed}"
    );
    assert_eq!(provider.times_asked(), 1, "a command went to the model");
}

#[tokio::test]
async fn the_tools_of_a_remote_server_are_actions_too_and_it_is_sent_its_key() {
    let home = tempfile::tempdir().unwrap();
    let key = ["remote", "key", "for", "the", "test"].join("-");
    Secrets::kept_in(home.path()).set("REMOTE_KEY", &key).unwrap();
    let server = MockServer::start().await;
    let bearer = format!("Bearer {key}");
    let authorised = || header("authorization", bearer.as_str());
    Mock::given(method("POST"))
        .and(authorised())
        .and(body_partial_json(json!({ "method": "initialize" })))
        .respond_with(|request: &wiremock::Request| {
            let asked: Value = serde_json::from_slice(&request.body).unwrap();
            ResponseTemplate::new(200).insert_header("mcp-session-id", "session-7").set_body_json(json!({
                "jsonrpc": "2.0", "id": asked["id"],
                "result": { "protocolVersion": "2025-06-18", "capabilities": {}, "serverInfo": { "name": "far", "version": "1" } },
            }))
        })
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "notifications/initialized" })))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(authorised())
        .and(header("mcp-session-id", "session-7"))
        .and(body_partial_json(json!({ "method": "tools/list" })))
        .respond_with(|request: &wiremock::Request| {
            let asked: Value = serde_json::from_slice(&request.body).unwrap();
            ResponseTemplate::new(200).set_body_json(
                json!({ "jsonrpc": "2.0", "id": asked["id"], "result": { "tools": [{
                "name": "lookup", "description": "Looks a word up.",
                "inputSchema": { "type": "object", "properties": { "word": { "type": "string" } } },
                "annotations": { "readOnlyHint": true },
            }] } }),
            )
        })
        .mount(&server)
        .await;
    // The answer to a call comes as a stream of events, as servers may choose.
    Mock::given(method("POST"))
        .and(authorised())
        .and(header("mcp-session-id", "session-7"))
        .and(body_partial_json(json!({ "method": "tools/call" })))
        .respond_with(|request: &wiremock::Request| {
            let asked: Value = serde_json::from_slice(&request.body).unwrap();
            let answer = json!({ "jsonrpc": "2.0", "id": asked["id"],
                "result": { "content": [{ "type": "text", "text": format!("{} means hello", asked["params"]["arguments"]["word"].as_str().unwrap()) }] } });
            let events = format!(
                "event: message\ndata: {}\n\nevent: message\ndata: {answer}\n\n",
                json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": {} })
            );
            ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string(events)
        })
        .mount(&server)
        .await;
    fs::write(
        home.path().join("mcp.json"),
        json!({ "mcpServers": { "far": { "url": server.uri(), "headers": { "Authorization": "Bearer ${REMOTE_KEY}" } } } }).to_string(),
    )
    .unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("far_lookup", json!({ "word": "salut" }))]).then(Ok("Looked up.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Look up salut.");
    owner.hears().await;

    assert_eq!(outcome(&provider), "salut means hello");
}

#[tokio::test]
async fn a_tool_never_stands_in_for_an_action_of_orbite_itself() {
    let home = home_with(json!({ "run": { "command": "python3", "args": [SERVER, "/dev/null"] } }));
    let provider = ScriptedProvider::answering(["Hi."]);
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Hi");
    owner.hears().await;

    let offered = provider.last_offered();
    let mut unique = offered.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), offered.len(), "{offered:?}");
    assert!(offered.contains(&"run_read_note".to_owned()), "{offered:?}");
}

#[tokio::test]
async fn a_server_that_stopped_is_started_again_for_the_next_call() {
    let home = home_with_notes();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("notes_read_note", json!({ "name": "crash" }))])
        .then_acts([("notes_read_note", json!({ "name": "shopping" }))])
        .then(Ok("Read.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Read my notes.");
    owner.hears().await;

    let told: Vec<String> = provider
        .last_asked()
        .into_iter()
        .filter_map(|message| match message {
            Message::Outcome { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert!(told[0].contains("notes") && told[0].contains("stopped"), "{}", told[0]);
    assert_eq!(told[1], "read-note shopping: token none\nsecond part");
}

#[tokio::test]
async fn a_local_server_that_speaks_todays_protocol_is_not_greeted_first() {
    let home = tempfile::tempdir().unwrap();
    let servers = json!({ "notes": { "command": "python3", "args": [SERVER, home.path().join("calls.log"), "modern"] } });
    fs::write(home.path().join("mcp.json"), json!({ "mcpServers": servers }).to_string()).unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("notes_read_note", json!({ "name": "shopping" }))])
        .then(Ok("Read.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Read my shopping note.");
    owner.hears().await;

    assert_eq!(outcome(&provider), "read-note shopping: token none\nsecond part");
}

#[tokio::test]
async fn a_remote_server_that_speaks_todays_protocol_is_told_what_each_request_is_for() {
    let home = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let today = || header("mcp-protocol-version", "2026-07-28");
    let stamped = json!({ "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } } });
    let answering = |result: Value| {
        move |request: &wiremock::Request| {
            let asked: Value = serde_json::from_slice(&request.body).unwrap();
            ResponseTemplate::new(200)
                .set_body_json(json!({ "jsonrpc": "2.0", "id": asked["id"], "result": result }))
        }
    };
    Mock::given(method("POST"))
        .and(today())
        .and(header("mcp-method", "server/discover"))
        .and(body_partial_json(stamped.clone()))
        .respond_with(answering(
            json!({ "resultType": "complete", "supportedVersions": ["2026-07-28"] }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(today())
        .and(header("mcp-method", "tools/list"))
        .and(body_partial_json(stamped.clone()))
        .respond_with(answering(
            json!({ "resultType": "complete", "ttlMs": 1000, "cacheScope": "private", "tools": [{
            "name": "lookup", "description": "Looks a word up.",
            "inputSchema": { "type": "object", "properties": { "word": { "type": "string" } } },
            "annotations": { "readOnlyHint": true },
        }] }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(today())
        .and(header("mcp-method", "tools/call"))
        .and(header("mcp-name", "lookup"))
        .and(body_partial_json(stamped))
        .respond_with(answering(
            json!({ "resultType": "complete", "content": [{ "type": "text", "text": "hello" }] }),
        ))
        .mount(&server)
        .await;
    fs::write(
        home.path().join("mcp.json"),
        json!({ "mcpServers": { "far": { "url": server.uri() } } }).to_string(),
    )
    .unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("far_lookup", json!({ "word": "salut" }))]).then(Ok("Looked up.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Look up salut.");
    owner.hears().await;

    assert_eq!(outcome(&provider), "hello");
    let greeted = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|request| String::from_utf8_lossy(&request.body).contains("initialize"));
    assert!(!greeted, "a server of today was greeted as an earlier one");
}

#[tokio::test]
async fn a_server_that_writes_without_end_is_cut_off_and_the_agent_carries_on() {
    let home = home_with_notes();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("notes_read_note", json!({ "name": "flood" }))])
        .then_acts([("notes_read_note", json!({ "name": "shopping" }))])
        .then(Ok("Read.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Read my notes.");
    tokio::time::timeout(std::time::Duration::from_secs(60), owner.hears_whenever()).await.unwrap();

    let told: Vec<String> = provider
        .last_asked()
        .into_iter()
        .filter_map(|message| match message {
            Message::Outcome { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert!(
        told[0].contains("more than") && told[0].len() < 1_000,
        "{}",
        &told[0][..told[0].len().min(300)]
    );
    assert_eq!(told[1], "read-note shopping: token none\nsecond part");
}

#[tokio::test]
async fn what_a_server_started_is_stopped_with_it() {
    let home = home_with_notes();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("notes_read_note", json!({ "name": "spawn" }))])
        .then_acts([("notes_read_note", json!({ "name": "crash" }))])
        .then(Ok("Done.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Go.");
    owner.hears().await;

    let started = provider
        .last_asked()
        .into_iter()
        .find_map(|message| match message {
            Message::Outcome { text, .. } => {
                text.lines().next().and_then(|pid| pid.parse::<u32>().ok())
            }
            _ => None,
        })
        .expect("the server did not say what it started");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let alive =
        std::fs::read(format!("/proc/{started}/cmdline")).is_ok_and(|command| !command.is_empty());
    assert!(!alive, "the program the server started is still running");
}

#[tokio::test]
async fn an_earlier_server_that_quits_at_a_request_it_does_not_know_is_started_again_and_greeted() {
    let home = tempfile::tempdir().unwrap();
    let servers = json!({ "notes": { "command": "python3", "args": [SERVER, home.path().join("calls.log"), "quits"] } });
    fs::write(home.path().join("mcp.json"), json!({ "mcpServers": servers }).to_string()).unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("notes_read_note", json!({ "name": "shopping" }))])
        .then(Ok("Read.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Read my shopping note.");
    owner.hears().await;

    assert_eq!(outcome(&provider), "read-note shopping: token none\nsecond part");
}

/// A remote server offering thirty tools, each described at length, as the
/// servers of large services do.
async fn a_server_with_many_tools() -> MockServer {
    let server = MockServer::start().await;
    let answering = |result: Value| {
        move |request: &wiremock::Request| {
            let asked: Value = serde_json::from_slice(&request.body).unwrap();
            ResponseTemplate::new(200)
                .set_body_json(json!({ "jsonrpc": "2.0", "id": asked["id"], "result": result }))
        }
    };
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "initialize" })))
        .respond_with(answering(json!({
            "protocolVersion": "2025-06-18", "capabilities": {},
            "serverInfo": { "name": "big", "version": "1" },
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "notifications/initialized" })))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    let tools: Vec<Value> = (0..30)
        .map(|number| {
            json!({
                "name": format!("tool-{number}"),
                "description": format!("Paints thing {number}. {}", "It has a great deal to say about itself. ".repeat(12)),
                "inputSchema": { "type": "object", "required": ["colour"], "properties": {
                    "colour": { "type": "string", "description": "The colour to paint it. ".repeat(6) },
                } },
            })
        })
        .collect();
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "tools/list" })))
        .respond_with(answering(json!({ "tools": tools })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "tools/call" })))
        .respond_with(|request: &wiremock::Request| {
            let asked: Value = serde_json::from_slice(&request.body).unwrap();
            let said = format!(
                "{} painted {}",
                asked["params"]["name"].as_str().unwrap(),
                asked["params"]["arguments"]["colour"].as_str().unwrap()
            );
            ResponseTemplate::new(200).set_body_json(json!({ "jsonrpc": "2.0", "id": asked["id"],
                "result": { "content": [{ "type": "text", "text": said }] } }))
        })
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_server_with_many_tools_is_offered_as_one_action_that_says_what_each_tool_takes() {
    let server = a_server_with_many_tools().await;
    let home = home_with(json!({ "big": { "url": server.uri() } }));
    let provider = ScriptedProvider::default();
    provider
        // Asked what a tool takes: nothing is done, and it says.
        .then_acts([("big", json!({ "tool": "tool-3" }))])
        .then_acts([("big", json!({ "tool": "tool-3", "input": { "colour": "red" } }))])
        .then(Ok("Painted.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Paint thing three red.");
    // Only the tool that acts asks; learning what it takes does not.
    let approval = owner.is_asked().await;
    assert!(approval.detail.contains("red"), "{approval:?}");
    owner.allows();
    assert_eq!(owner.hears().await, "Painted.");

    // One Action stands for the thirty, so that every turn does not carry them all.
    let offered = provider.last_offered();
    assert!(offered.contains(&"big".to_owned()), "{offered:?}");
    assert!(!offered.iter().any(|name| name.starts_with("big_")), "{offered:?}");
    let told: Vec<String> = provider
        .last_asked()
        .into_iter()
        .filter_map(|message| match message {
            Message::Outcome { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert!(told[0].contains("colour") && told[0].contains("Paints thing 3"), "{}", told[0]);
    assert_eq!(told[1], "tool-3 painted red");
    let called = server.received_requests().await.unwrap();
    let calls = called
        .iter()
        .filter(|request| String::from_utf8_lossy(&request.body).contains("tools/call"));
    assert_eq!(calls.count(), 1, "asking what a tool takes called the server");
}

#[tokio::test]
async fn a_tool_that_does_not_exist_on_such_a_server_is_said_with_those_that_do() {
    let server = a_server_with_many_tools().await;
    let home = home_with(json!({ "big": { "url": server.uri() } }));
    let provider = ScriptedProvider::default();
    provider.then_acts([("big", json!({ "tool": "tool-99" }))]).then(Ok("Sorry.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Paint thing ninety-nine.");
    owner.hears().await;

    let told = outcome(&provider);
    assert!(told.contains("tool-99") && told.contains("tool-0"), "{told}");
}
