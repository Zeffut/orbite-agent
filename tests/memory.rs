//! What an Owner observes of the Agent's Memory.

mod support;

use std::fs;
use std::path::Path;

use orbite::{Agent, Message, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};

/// Starts an Agent keeping its Memory in `memory`, thinking with `provider`.
fn agent_remembering_in(memory: &Path, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let memory = memory.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory()).remembering_in(memory).converse(&mut channel).await
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
async fn a_fact_told_today_is_known_after_a_restart_and_a_new_conversation() {
    let memory = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("remember", json!({ "name": "home", "text": "Lives in Lyon." }))])
        .then(Ok("Noted.".into()));
    let mut owner = agent_remembering_in(memory.path(), &provider);
    owner.says("I live in Lyon.");
    assert_eq!(owner.hears().await, "Noted.");
    assert!(owner.was_asked_nothing(), "remembering needs no Approval");
    drop(owner);

    // A new process with an empty Conversation: only Memory carries over.
    let provider = ScriptedProvider::answering(["Lyon."]);
    let mut owner = agent_remembering_in(memory.path(), &provider);
    owner.says("Where do I live?");
    owner.hears().await;

    assert!(
        provider.last_instructions().contains("Lives in Lyon."),
        "{}",
        provider.last_instructions()
    );
}

#[tokio::test]
async fn each_memory_is_a_file_the_owner_can_read() {
    let memory = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([(
            "remember",
            json!({ "name": "Coffee order", "text": "Flat white, no sugar." }),
        )])
        .then(Ok("Noted.".into()));
    let mut owner = agent_remembering_in(memory.path(), &provider);

    owner.says("I take a flat white, no sugar.");
    owner.hears().await;

    assert_eq!(
        fs::read_to_string(memory.path().join("coffee-order.md")).unwrap(),
        "Flat white, no sugar.\n"
    );
}

#[tokio::test]
async fn the_model_is_shown_every_memory_by_name() {
    let memory = tempfile::tempdir().unwrap();
    fs::write(memory.path().join("home.md"), "Lives in Lyon.\n").unwrap();
    fs::write(memory.path().join("work.md"), "Is a nurse.\n").unwrap();
    let provider = ScriptedProvider::answering(["You live in Lyon and are a nurse."]);
    let mut owner = agent_remembering_in(memory.path(), &provider);

    owner.says("What do you remember about me?");
    owner.hears().await;

    let instructions = provider.last_instructions();
    assert!(instructions.contains("home: Lives in Lyon."), "{instructions}");
    assert!(instructions.contains("work: Is a nurse."), "{instructions}");
}

#[tokio::test]
async fn what_the_owner_changes_by_hand_is_what_the_agent_knows() {
    let memory = tempfile::tempdir().unwrap();
    fs::write(memory.path().join("home.md"), "Lives in Lyon.\n").unwrap();
    fs::write(memory.path().join("work.md"), "Is a nurse.\n").unwrap();
    let provider = ScriptedProvider::answering(["Ok.", "Ok."]);
    let mut owner = agent_remembering_in(memory.path(), &provider);
    owner.says("Hi");
    owner.hears().await;

    fs::write(memory.path().join("home.md"), "Lives in Nantes.\n").unwrap();
    fs::remove_file(memory.path().join("work.md")).unwrap();
    owner.says("And now?");
    owner.hears().await;

    let instructions = provider.last_instructions();
    assert!(instructions.contains("Lives in Nantes."), "{instructions}");
    assert!(!instructions.contains("Lyon") && !instructions.contains("nurse"), "{instructions}");
}

#[tokio::test]
async fn remembering_under_the_same_name_corrects_the_memory() {
    let memory = tempfile::tempdir().unwrap();
    fs::write(memory.path().join("home.md"), "Lives in Lyon.\n").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("remember", json!({ "name": "home", "text": "Lives in Nantes since 2026." }))])
        .then(Ok("Corrected.".into()));
    let mut owner = agent_remembering_in(memory.path(), &provider);

    owner.says("No, I moved to Nantes.");
    owner.hears().await;

    assert_eq!(
        fs::read_to_string(memory.path().join("home.md")).unwrap(),
        "Lives in Nantes since 2026.\n"
    );
    assert_eq!(fs::read_dir(memory.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn a_memory_the_owner_wants_gone_is_deleted() {
    let memory = tempfile::tempdir().unwrap();
    fs::write(memory.path().join("home.md"), "Lives in Lyon.\n").unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("forget", json!({ "name": "home" }))]).then(Ok("Forgotten.".into()));
    let mut owner = agent_remembering_in(memory.path(), &provider);

    owner.says("Forget where I live.");
    assert_eq!(owner.hears().await, "Forgotten.");

    assert!(!memory.path().join("home.md").exists());
    assert!(!provider.last_instructions().contains("Lyon"));
}

#[tokio::test]
async fn forgetting_what_is_not_remembered_tells_the_model_what_is() {
    let memory = tempfile::tempdir().unwrap();
    fs::write(memory.path().join("home.md"), "Lives in Lyon.\n").unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("forget", json!({ "name": "address" }))]).then(Ok("Hm.".into()));
    let mut owner = agent_remembering_in(memory.path(), &provider);

    owner.says("Forget my address.");
    owner.hears().await;

    assert!(outcome(&provider).contains("home"), "{}", outcome(&provider));
    assert!(memory.path().join("home.md").exists());
}

#[tokio::test]
async fn a_memory_name_cannot_reach_outside_the_memory_folder() {
    let home = tempfile::tempdir().unwrap();
    let memory = home.path().join("memory");
    fs::write(home.path().join("precious.md"), "do not touch").unwrap();
    for (action, input) in [
        ("remember", json!({ "name": "../precious", "text": "overwritten" })),
        ("forget", json!({ "name": "../precious" })),
        ("remember", json!({ "name": "/etc/cron.d/evil", "text": "x" })),
        ("remember", json!({ "name": "...", "text": "x" })),
    ] {
        let provider = ScriptedProvider::default();
        provider.then_acts([(action, input)]).then(Ok("Ok.".into()));
        let mut owner = agent_remembering_in(&memory, &provider);
        owner.says("Go.");
        owner.hears().await;
    }

    assert_eq!(fs::read_to_string(home.path().join("precious.md")).unwrap(), "do not touch");
    for entry in fs::read_dir(home.path()).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(name == "memory" || name == "precious.md", "{name:?} was created outside");
    }
}

#[tokio::test]
async fn an_agent_with_nowhere_to_remember_is_not_offered_to() {
    let provider = ScriptedProvider::answering(["Hi."]);
    let (mut channel, mut owner) = memory_channel();
    let agent_provider = provider.clone();
    tokio::spawn(async move {
        Agent::new(agent_provider, Store::in_memory()).converse(&mut channel).await
    });

    owner.says("Hi");
    owner.hears().await;

    assert!(!provider.last_offered().contains(&"remember".to_string()));
    let mut owner_with_memory = {
        let memory = tempfile::tempdir().unwrap();
        let provider_with = ScriptedProvider::answering(["Hi."]);
        let mut owner = agent_remembering_in(memory.path(), &provider_with);
        owner.says("Hi");
        owner.hears().await;
        assert!(provider_with.last_offered().contains(&"remember".to_string()));
        owner
    };
    assert!(owner_with_memory.was_asked_nothing());
}
