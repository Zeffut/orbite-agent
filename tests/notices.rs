//! What an Owner hears of what Orbite itself did while they were not
//! looking, such as updating: the Agent passes it on.

mod support;

use std::fs;
use std::time::Duration;

use orbite::{Agent, Message, Store};
use support::{ScriptedProvider, memory_channel, saying};

/// A model that passes on the last notice it was given.
fn relaying() -> ScriptedProvider {
    let provider = ScriptedProvider::default();
    provider.replying_otherwise(|_, conversation| match conversation.last() {
        Some(Message::Notice(text)) => Ok(saying(&format!("By the way: {text}"))),
        _ => Ok(saying("Hello.")),
    });
    provider
}

#[tokio::test]
async fn the_agent_passes_on_what_orbite_left_for_the_owner_and_only_once() {
    let home = tempfile::tempdir().unwrap();
    let notices = home.path().join("notices");
    let (mut channel, mut owner) = memory_channel();
    let provider = relaying();
    let folder = notices.clone();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .hearing_notices_in(folder)
            .checking_time_every(Duration::from_millis(15))
            .converse(&mut channel)
            .await
    });

    fs::create_dir_all(&notices).unwrap();
    fs::write(notices.join("1-update.txt"), "Orbite was updated to version 0.2.0.").unwrap();

    let told = owner.hears().await;
    assert!(told.contains("updated to version 0.2.0"), "{told}");
    assert!(owner.hears_nothing().await, "the same notice was passed on twice");
    assert_eq!(fs::read_dir(&notices).unwrap().count(), 0, "the notice was kept after being read");
}
