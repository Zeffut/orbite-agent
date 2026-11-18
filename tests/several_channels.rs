//! What an Owner observes when the one Conversation is open in several
//! places at once.

mod support;

use std::path::Path;
use std::time::Duration;

use orbite::{Agent, Both, Message, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};

/// Starts an Agent whose Conversation is open in two places; the Owner's
/// view of each.
fn agent_in_two_places(workspace: &Path, provider: &ScriptedProvider) -> (Owner, Owner) {
    let (phone_channel, phone) = memory_channel();
    let (desk_channel, desk) = memory_channel();
    let provider = provider.clone();
    let workspace = workspace.to_owned();
    tokio::spawn(async move {
        let mut everywhere = Both::new(phone_channel, desk_channel);
        Agent::new(provider, Store::in_memory())
            .working_in(workspace)
            .stopping_commands_after(Duration::from_secs(20))
            .converse(&mut everywhere)
            .await
    });
    (phone, desk)
}

#[tokio::test]
async fn what_is_written_in_one_place_is_seen_and_answered_in_every_place() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Hello from your agent.", "Still here."]);
    let (mut phone, mut desk) = agent_in_two_places(workspace.path(), &provider);

    phone.says("Hi");

    assert_eq!(phone.hears().await, "Hello from your agent.");
    assert_eq!(desk.hears().await, "(you, elsewhere) Hi", "the other place shows what was written");
    assert_eq!(desk.hears().await, "Hello from your agent.");

    desk.says("And from here?");
    assert_eq!(desk.hears().await, "Still here.");
    assert_eq!(phone.hears().await, "(you, elsewhere) And from here?");
    assert_eq!(phone.hears().await, "Still here.");
    assert_eq!(
        provider.last_asked(),
        [
            Message::from_owner("Hi"),
            Message::from_agent("Hello from your agent."),
            Message::from_owner("And from here?"),
        ],
        "it is one Conversation"
    );
}

#[tokio::test]
async fn an_approval_is_asked_everywhere_and_answered_once_from_anywhere() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "touch made.txt" }))])
        .then(Ok("Created.".into()));
    let (mut phone, mut desk) = agent_in_two_places(workspace.path(), &provider);
    phone.says("Create it.");
    desk.hears().await;

    let on_phone = phone.is_asked().await;
    let on_desk = desk.is_asked().await;
    assert_eq!(on_phone, on_desk);
    desk.allows();

    assert_eq!(
        phone.hears().await,
        "(answered elsewhere) Allowed",
        "the other place closes its question"
    );
    assert_eq!(phone.hears().await, "Created.");
    assert_eq!(desk.hears().await, "Created.");
    assert!(workspace.path().join("made.txt").exists());
    assert!(phone.was_asked_nothing() && desk.was_asked_nothing(), "it was asked once");
}

#[tokio::test]
async fn the_steps_of_the_work_and_files_reach_every_place() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("notes.txt"), "milk").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("read_file", json!({ "path": "notes.txt" }))])
        .then_acts([("send_file", json!({ "path": "notes.txt", "caption": "Yours." }))])
        .then(Ok("Done.".into()));
    let (mut phone, mut desk) = agent_in_two_places(workspace.path(), &provider);

    phone.says("Read my notes and send them.");

    assert_eq!(phone.hears().await, "(file notes.txt: milk) Yours.");
    assert_eq!(phone.hears().await, "Done.");
    desk.hears().await;
    assert_eq!(desk.hears().await, "(file notes.txt: milk) Yours.");
    assert_eq!(desk.hears().await, "Done.");
    tokio::task::yield_now().await;
    assert_eq!(phone.saw(), ["📄 Reading notes.txt", "📎 Sending notes.txt", "(idle)"]);
    assert_eq!(desk.saw(), phone.saw());
}

#[tokio::test]
async fn the_agent_can_be_stopped_from_another_place_than_where_it_was_asked() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.then_never_answers();
    let (mut phone, mut desk) = agent_in_two_places(workspace.path(), &provider);
    phone.says("Think hard.");
    desk.hears().await;
    while provider.times_asked() == 0 {
        tokio::task::yield_now().await;
    }

    desk.says("stop");

    assert_eq!(desk.hears().await, "Stopped.");
    assert_eq!(phone.hears().await, "(you, elsewhere) stop");
    assert_eq!(phone.hears().await, "Stopped.");
}

#[tokio::test]
async fn the_conversation_goes_on_when_one_place_closes() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["One.", "Two."]);
    let (mut phone, mut desk) = agent_in_two_places(workspace.path(), &provider);
    phone.says("First");
    phone.hears().await;
    desk.hears().await;
    desk.hears().await;

    drop(desk);
    phone.says("Second");

    assert_eq!(phone.hears().await, "Two.");
}

#[tokio::test]
async fn a_place_opened_later_can_read_the_conversation_so_far() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("orbite.db");
    {
        let provider = ScriptedProvider::answering(["Noted.", "Again."]);
        let (mut channel, mut owner) = memory_channel();
        let store = Store::open(&database).unwrap();
        tokio::spawn(async move { Agent::new(provider, store).converse(&mut channel).await });
        owner.says("I live in Lyon.");
        owner.hears().await;
        owner.says("Once more.");
        owner.hears().await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let store = Store::open(&database).unwrap();
    let everything = store.history(None, 100).unwrap();
    let said: Vec<&Message> = everything.iter().map(|entry| &entry.message).collect();
    assert_eq!(
        said,
        [
            &Message::from_owner("I live in Lyon."),
            &Message::from_agent("Noted."),
            &Message::from_owner("Once more."),
            &Message::from_agent("Again."),
        ]
    );
    assert!(everything.iter().all(|entry| entry.at > 0), "each entry carries when it was said");

    // Only what came after what the place already has.
    let later = store.history(Some(everything[1].id), 100).unwrap();
    assert_eq!(later.len(), 2);
    assert_eq!(later[0].message, Message::from_owner("Once more."));
    // And no more than asked for, the latest first to be kept.
    let latest = store.history(None, 1).unwrap();
    assert_eq!(latest[0].message, Message::from_agent("Again."));
}
