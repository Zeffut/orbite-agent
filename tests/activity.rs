//! What an Owner is shown of the Agent's work while it works.

mod support;

use orbite::{Agent, Store};
use serde_json::json;
use support::{ScriptedProvider, memory_channel};

#[tokio::test]
async fn each_step_is_shown_as_it_starts_and_cleared_when_the_work_is_over() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("notes.txt"), "milk").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "ls -la" }))])
        .then_acts([
            ("read_file", json!({ "path": "notes.txt" })),
            ("search_web", json!({ "query": "milk price" })),
            ("read_page", json!({ "url": "https://example.invalid/prices?id=3" })),
            ("remember", json!({ "name": "shopping", "text": "Buys milk." })),
        ])
        .then(Ok("Done.".into()));
    let (mut channel, mut owner) = memory_channel();
    let folder = home.path().to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .working_in(&folder)
            .remembering_in(folder.join("memory"))
            .searching_at("http://127.0.0.1:1")
            .converse(&mut channel)
            .await
    });

    owner.says("Check my notes and the price of milk.");
    assert_eq!(owner.hears().await, "Done.");
    tokio::task::yield_now().await;

    assert_eq!(
        owner.saw(),
        [
            "💻 Running: ls -la",
            "📄 Reading notes.txt",
            "🔎 Searching the web: milk price",
            "🌐 Reading example.invalid",
            "🧠 Remembering: shopping",
            "(idle)",
        ]
    );
}

#[tokio::test]
async fn an_action_that_waits_for_an_approval_is_shown_only_once_allowed() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("write_file", json!({ "path": "a.txt", "content": "x" }))])
        .then_acts([("write_file", json!({ "path": "b.txt", "content": "x" }))])
        .then(Ok("Done.".into()));
    let (mut channel, mut owner) = memory_channel();
    let folder = home.path().to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory()).working_in(folder).converse(&mut channel).await
    });

    owner.says("Write both.");
    owner.is_asked().await;
    assert!(
        owner.saw().is_empty(),
        "nothing is under way while the Approval waits: {:?}",
        owner.saw()
    );
    owner.allows();
    owner.is_asked().await;
    owner.denies();
    owner.hears().await;
    tokio::task::yield_now().await;

    assert_eq!(owner.saw(), ["✍️ Writing a.txt", "(idle)"]);
}

#[tokio::test]
async fn a_long_command_is_shown_shortened() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    let long = format!("echo {}", "word ".repeat(60));
    provider.then_acts([("run_command", json!({ "command": long }))]).then(Ok("Done.".into()));
    let (mut channel, mut owner) = memory_channel();
    let folder = home.path().to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory()).working_in(folder).converse(&mut channel).await
    });

    owner.says("Go.");
    owner.hears().await;

    let shown = &owner.saw()[0];
    assert!(shown.chars().count() <= 90 && shown.ends_with('…'), "{shown}");
}
