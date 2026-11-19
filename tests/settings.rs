//! What an Owner observes of the settings: one menu in the Conversation,
//! the same values from the `orbite` command, and each change seen in both.

mod support;

use std::path::Path;
use std::process::Command;

use orbite::{Agent, Config, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};

fn agent_in(home: &Path, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let home = home.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::open(home.join("orbite.db")).unwrap())
            .working_in(home.join("workspace"))
            .configured_in(&home)
            .converse(&mut channel)
            .await
    });
    owner
}

fn orbite(home: &Path, arguments: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_orbite"))
        .args(arguments)
        .env("ORBITE_HOME", home)
        .output()
        .unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[tokio::test]
async fn the_menu_shows_every_setting_with_its_value_and_how_to_change_it() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    let mut owner = agent_in(home.path(), &provider);

    owner.says("/settings");
    let menu = owner.hears().await;

    for expected in [
        "careful",
        "/settings permission",
        "/settings quiet",
        "/settings wakeups",
        "/settings secrets",
        "/settings updates",
        "/settings language",
    ] {
        assert!(menu.contains(expected), "{expected} is missing from:\n{menu}");
    }
    assert_eq!(provider.times_asked(), 0, "the menu went to the model");
}

#[tokio::test]
async fn a_setting_shows_its_choices_and_a_choice_changes_it() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("write_file", json!({ "path": "note.txt", "content": "x" }))])
        .then(Ok("Written.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("/settings permission");
    let choices = owner.hears().await;
    assert!(
        choices.contains("/settings permission balanced")
            && choices.contains("/settings permission free"),
        "{choices}"
    );

    owner.says("/settings permission balanced");
    let menu = owner.hears().await;
    assert!(menu.contains("balanced"), "{menu}");

    // It is in force: writing in its own folder no longer asks.
    owner.says("Write a note.");
    assert_eq!(owner.hears().await, "Written.");
    assert!(owner.was_asked_nothing());
}

#[tokio::test]
async fn a_value_that_makes_no_sense_is_refused_with_what_is_possible() {
    let home = tempfile::tempdir().unwrap();
    let mut owner = agent_in(home.path(), &ScriptedProvider::default());

    owner.says("/settings quiet whenever");
    let refused = owner.hears().await;
    assert!(refused.contains("HH:MM"), "{refused}");

    owner.says("/settings quiet 22:30-07:00");
    assert!(owner.hears().await.contains("22:30-07:00"));
}

#[tokio::test]
async fn what_is_changed_in_the_conversation_is_seen_by_the_command_and_the_other_way() {
    let home = tempfile::tempdir().unwrap();
    let mut owner = agent_in(home.path(), &ScriptedProvider::default());

    owner.says("/settings wakeups 60");
    owner.hears().await;
    owner.says("/settings updates off");
    owner.hears().await;
    let listed = orbite(home.path(), &["settings"]);
    assert!(listed.contains("wakeups = 60") && listed.contains("updates = off"), "{listed}");
    assert_eq!(Config::load(home.path()).unwrap().automatic_updates, Some(false));

    let set = orbite(home.path(), &["settings", "permission", "free"]);
    assert!(set.contains("permission = free"), "{set}");
    orbite(home.path(), &["settings", "language", "fr"]);
    owner.says("/settings");
    let menu = owner.hears().await;
    assert!(menu.contains("libre") && menu.contains("Réglages"), "{menu}");
}

#[test]
fn the_command_refuses_what_makes_no_sense() {
    let home = tempfile::tempdir().unwrap();
    let refused = orbite(home.path(), &["settings", "permission", "reckless"]);
    assert!(refused.contains("careful") && refused.contains("free"), "{refused}");
    assert!(orbite(home.path(), &["settings"]).contains("permission = careful"));
}
