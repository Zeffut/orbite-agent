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

#[tokio::test]
async fn the_model_is_changed_from_the_conversation_and_orbite_restarts_to_take_it() {
    let home = tempfile::tempdir().unwrap();
    let chosen = orbite::ProviderChoice {
        name: "groq".into(),
        model: "first-model".into(),
        ..Default::default()
    };
    Config { provider: Some(chosen), ..Config::default() }.save(home.path()).unwrap();
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(home.path().join("orbite.db")).unwrap();
    let folder = home.path().to_owned();
    let running = tokio::spawn(async move {
        Agent::new(ScriptedProvider::default(), store)
            .configured_in(&folder)
            .restarted_by_its_service()
            .converse(&mut channel)
            .await
    });

    owner.says("/settings");
    assert!(owner.hears().await.contains("first-model"));
    owner.says("/settings model second-model");
    let said = owner.hears().await;

    assert!(said.contains("second-model") && said.to_lowercase().contains("restart"), "{said}");
    assert_eq!(Config::load(home.path()).unwrap().provider.unwrap().model, "second-model");
    // The Agent ends so that its service starts it again with the new model.
    tokio::time::timeout(std::time::Duration::from_secs(2), running)
        .await
        .expect("it kept running")
        .unwrap();
    assert!(orbite(home.path(), &["settings"]).contains("model = second-model"));
}

#[tokio::test]
async fn without_a_service_to_restart_it_the_agent_says_when_the_change_applies() {
    let home = tempfile::tempdir().unwrap();
    let chosen = orbite::ProviderChoice {
        name: "groq".into(),
        model: "first-model".into(),
        ..Default::default()
    };
    Config { provider: Some(chosen), ..Config::default() }.save(home.path()).unwrap();
    let mut owner = agent_in(home.path(), &ScriptedProvider::answering(["Still here."]));

    owner.says("/settings model second-model");
    let said = owner.hears().await;
    assert!(said.contains("next start"), "{said}");

    owner.says("Hello");
    assert_eq!(owner.hears().await, "Still here.");
}

#[tokio::test]
async fn what_writes_voice_messages_down_is_seen_and_switched_from_the_conversation() {
    let home = tempfile::tempdir().unwrap();
    // Set up on the machine earlier: whisper.cpp with a model.
    let local = orbite::Transcription {
        engine: "whisper.cpp".into(),
        model: Some("small".into()),
        ..Default::default()
    };
    Config { transcription: Some(local), ..Config::default() }.save(home.path()).unwrap();
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(home.path().join("orbite.db")).unwrap();
    let folder = home.path().to_owned();
    tokio::spawn(async move {
        Agent::new(ScriptedProvider::default(), store)
            .configured_in(&folder)
            .converse(&mut channel)
            .await
    });

    owner.says("/settings");
    let shown = owner.hears().await;
    assert!(shown.contains("whisper.cpp (small)"), "{shown}");

    owner.says("/settings voice provider");
    let said = owner.hears().await;
    assert!(said.contains("provider"), "{said}");
    let kept = Config::load(home.path()).unwrap().transcription.unwrap();
    // The model chosen on the machine is kept, to go back to it.
    assert_eq!((kept.engine.as_str(), kept.model.as_deref()), ("provider", Some("small")));

    owner.says("/settings voice whisper.cpp");
    owner.hears().await;
    assert_eq!(Config::load(home.path()).unwrap().transcription.unwrap().engine, "whisper.cpp");

    // What was never set up on the machine is not switched to from here.
    owner.says("/settings voice server");
    let refused = owner.hears().await;
    assert!(refused.contains("orbite transcription"), "{refused}");
    assert_eq!(Config::load(home.path()).unwrap().transcription.unwrap().engine, "whisper.cpp");
}

#[tokio::test]
async fn the_agents_name_and_its_model_for_light_work_are_changed_from_the_conversation() {
    let home = tempfile::tempdir().unwrap();
    let chosen = orbite::ProviderChoice {
        name: "groq".into(),
        model: "first-model".into(),
        ..Default::default()
    };
    Config { provider: Some(chosen), agent_name: Some("Ada".into()), ..Config::default() }
        .save(home.path())
        .unwrap();
    let converse = |home: &std::path::Path| {
        let (mut channel, owner) = memory_channel();
        let store = Store::open(home.join("orbite.db")).unwrap();
        let folder = home.to_owned();
        let running = tokio::spawn(async move {
            Agent::new(ScriptedProvider::default(), store)
                .configured_in(&folder)
                .restarted_by_its_service()
                .converse(&mut channel)
                .await
        });
        (owner, running)
    };

    let (mut owner, running) = converse(home.path());
    owner.says("/settings");
    let shown = owner.hears().await;
    assert!(shown.contains("Ada") && shown.contains("Model for light work"), "{shown}");
    // A name of several words, capitals kept.
    owner.says("/settings name Marie Curie");
    let said = owner.hears().await;
    assert!(said.contains("Marie Curie") && said.to_lowercase().contains("restart"), "{said}");
    assert_eq!(Config::load(home.path()).unwrap().agent_name.as_deref(), Some("Marie Curie"));
    let _ = running.await;

    let (mut owner, running) = converse(home.path());
    owner.says("/settings light Quick-Model");
    assert!(owner.hears().await.contains("Quick-Model"));
    assert_eq!(Config::load(home.path()).unwrap().light_model.as_deref(), Some("Quick-Model"));
    let _ = running.await;

    let (mut owner, _running) = converse(home.path());
    owner.says("/settings light off");
    owner.hears().await;
    assert_eq!(Config::load(home.path()).unwrap().light_model, None);
    // The command on the machine reads the same.
    let listed = orbite(home.path(), &["settings"]);
    assert!(listed.contains("name = Marie Curie") && listed.contains("light = off"), "{listed}");
}

#[tokio::test]
async fn asked_in_plain_words_the_agent_adds_an_ai_and_its_key_is_given_in_the_conversation() {
    let home = tempfile::tempdir().unwrap();
    let chosen = orbite::ProviderChoice {
        name: "groq".into(),
        model: "first-model".into(),
        ..Default::default()
    };
    Config { provider: Some(chosen), ..Config::default() }.save(home.path()).unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([(
            "command",
            serde_json::json!({ "command": "/providers add mistral mistral-large" }),
        )])
        .then(Ok("[silent]".into()));
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(home.path().join("orbite.db")).unwrap();
    let (folder, model) = (home.path().to_owned(), provider.clone());
    let running = tokio::spawn(async move {
        Agent::new(model, store)
            .configured_in(&folder)
            .keeping_secrets_in(&folder)
            .restarted_by_its_service()
            .converse(&mut channel)
            .await
    });

    owner.says("Add Mistral as a backup AI, with its large model.");
    // Who the Agent thinks with is the Owner's to agree to.
    let asked = owner.is_asked().await;
    assert!(asked.summary.contains("Add the AI mistral"), "{}", asked.summary);
    owner.allows();
    // Its key is asked for in the conversation, and never shown to a model.
    let wanted = owner.hears().await;
    assert!(wanted.contains("API key") && wanted.contains("Mistral"), "{wanted}");
    owner.says("mk-a-key-made-up-for-this-test");
    let added = owner.hears().await;
    assert!(added.contains("mistral is added"), "{added}");
    assert!(!added.contains("mk-a-key"), "{added}");
    // Orbite starts again to think with it too.
    tokio::time::timeout(std::time::Duration::from_secs(2), running)
        .await
        .expect("it kept running")
        .unwrap();

    let all = Config::load(home.path()).unwrap().providers();
    assert_eq!(all.len(), 2);
    assert_eq!((all[1].name.as_str(), all[1].model.as_str()), ("mistral", "mistral-large"));
    let key = all[1].key.clone().expect("it has a key of its own");
    assert_eq!(
        orbite::Secrets::kept_in(home.path()).get(&key).unwrap().as_deref(),
        Some("mk-a-key-made-up-for-this-test")
    );
    let shown = format!("{:?}", provider.every_call());
    assert!(!shown.contains("mk-a-key"), "the key was shown to a model: {shown}");

    // Back after the restart, nothing is awaited any more: what the Owner
    // says next is an ordinary message, and the key stays the one they gave.
    let after = ScriptedProvider::answering(["Hello."]);
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(home.path().join("orbite.db")).unwrap();
    let (folder, model) = (home.path().to_owned(), after.clone());
    tokio::spawn(async move {
        Agent::new(model, store)
            .configured_in(&folder)
            .keeping_secrets_in(&folder)
            .converse(&mut channel)
            .await
    });
    owner.says("Hi there");
    assert_eq!(owner.hears().await, "Hello.");
    assert_eq!(
        orbite::Secrets::kept_in(home.path()).get(&key).unwrap().as_deref(),
        Some("mk-a-key-made-up-for-this-test")
    );
}
