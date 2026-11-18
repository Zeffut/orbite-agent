//! What an Owner observes when Orbite stops and starts again.

mod support;

use orbite::{Agent, Message, ProviderError, Store};
use support::{ScriptedProvider, memory_channel};

#[tokio::test]
async fn after_a_restart_the_agent_still_knows_what_was_said() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("orbite.db");

    {
        let provider = ScriptedProvider::answering(["Noted."]);
        let (mut channel, mut owner) = memory_channel();
        let store = Store::open(&database).unwrap();
        let running =
            tokio::spawn(async move { Agent::new(provider, store).converse(&mut channel).await });
        owner.says("I live in Lyon.");
        owner.hears().await;
        drop(owner);
        running.await.unwrap();
    }

    let provider = ScriptedProvider::answering(["Lyon."]);
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(&database).unwrap();
    let agent_provider = provider.clone();
    tokio::spawn(async move {
        Agent::new(agent_provider, store)
            .converse(&mut channel)
            .await
    });
    owner.says("Where do I live?");
    owner.hears().await;

    assert_eq!(
        provider.last_asked(),
        [
            Message::from_owner("I live in Lyon."),
            Message::from_agent("Noted."),
            Message::from_owner("Where do I live?"),
        ]
    );
}

#[tokio::test]
async fn a_message_left_unanswered_by_a_stop_is_answered_at_the_next_start() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("orbite.db");

    let provider = ScriptedProvider::default();
    provider.then_never_answers();
    let (mut channel, owner) = memory_channel();
    let store = Store::open(&database).unwrap();
    let asked = provider.clone();
    let running =
        tokio::spawn(async move { Agent::new(provider, store).converse(&mut channel).await });
    owner.says("Remind me to call Ana.");
    while asked.times_asked() == 0 {
        tokio::task::yield_now().await;
    }
    running.abort();
    let _ = running.await;

    let provider = ScriptedProvider::answering(["I will."]);
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(&database).unwrap();
    tokio::spawn(async move { Agent::new(provider, store).converse(&mut channel).await });

    assert_eq!(owner.hears().await, "I will.");
}

#[tokio::test]
async fn a_message_the_provider_failed_on_is_not_answered_again_after_a_restart() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("orbite.db");

    {
        let provider = ScriptedProvider::default();
        provider.then(Err(ProviderError::Unreachable("connection refused".into())));
        let (mut channel, mut owner) = memory_channel();
        let store = Store::open(&database).unwrap();
        let running =
            tokio::spawn(async move { Agent::new(provider, store).converse(&mut channel).await });
        owner.says("Hi");
        owner.hears().await;
        drop(owner);
        running.await.unwrap();
    }

    let provider = ScriptedProvider::answering(["Never said."]);
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(&database).unwrap();
    tokio::spawn(async move { Agent::new(provider, store).converse(&mut channel).await });

    assert!(owner.hears_nothing().await);
}

#[test]
fn a_database_from_a_newer_orbite_is_refused_rather_than_damaged() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("orbite.db");
    rusqlite::Connection::open(&database)
        .unwrap()
        .pragma_update(None, "user_version", 9999)
        .unwrap();

    let error = Store::open(&database)
        .err()
        .expect("the Store should refuse to open");

    assert!(error.to_string().contains("newer Orbite"), "{error}");
}

#[tokio::test]
async fn a_conversation_from_the_first_version_of_orbite_is_kept() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("orbite.db");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "CREATE TABLE conversation (
                 id      INTEGER PRIMARY KEY AUTOINCREMENT,
                 speaker TEXT NOT NULL CHECK (speaker IN ('owner', 'agent')),
                 text    TEXT NOT NULL
             ) STRICT;
             INSERT INTO conversation (speaker, text) VALUES ('owner', 'I live in Lyon.'), ('agent', 'Noted.');
             PRAGMA user_version = 1;",
        )
        .unwrap();

    let provider = ScriptedProvider::answering(["Lyon."]);
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(&database).unwrap();
    let agent_provider = provider.clone();
    tokio::spawn(async move {
        Agent::new(agent_provider, store)
            .converse(&mut channel)
            .await
    });
    owner.says("Where do I live?");
    owner.hears().await;

    assert_eq!(
        provider.last_asked(),
        [
            Message::from_owner("I live in Lyon."),
            Message::from_agent("Noted."),
            Message::from_owner("Where do I live?"),
        ]
    );
}
