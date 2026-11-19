//! What an Owner observes when the Agent writes Extensions for itself: it
//! always asks first, what it wrote works at once, it can be repaired, and
//! it can be undone with everything it kept.

mod support;

use std::fs;
use std::path::Path;

use orbite::{Agent, Message, Store};
use serde_json::{Value, json};
use support::{Owner, ScriptedProvider, memory_channel};

/// An Extension that counts how often it was asked.
const COUNTER: &str = r#"
function describe() {
  return [{ name: "bump", description: "Counts one more.", parameters: {} }];
}
function run() {
  const count = (store.get("count") || 0) + 1;
  store.set("count", count);
  return "Count: " + count;
}
"#;

fn counter(script: &str) -> Value {
    json!({ "name": "counter", "description": "Counts things.", "hosts": [], "script": script })
}

fn agent_extended_in(extensions: &Path, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let extensions = extensions.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .extending_in(extensions)
            .converse(&mut channel)
            .await
    });
    owner
}

fn outcomes(provider: &ScriptedProvider) -> Vec<String> {
    provider
        .last_asked()
        .into_iter()
        .filter_map(|message| match message {
            Message::Outcome { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn what_the_agent_writes_is_shown_to_the_owner_and_works_at_once() {
    let extensions = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("create_extension", counter(COUNTER))])
        .then_acts([("counter_bump", json!({}))])
        .then_acts([("counter_bump", json!({}))])
        .then(Ok("It counts.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Make yourself a counter.");
    let asked = owner.is_asked().await;
    assert!(asked.critical, "creating an Extension is asked about at every Permission Level");
    assert!(
        asked.detail.contains("counter") && asked.detail.contains("Counts things."),
        "{asked:?}"
    );
    assert!(asked.detail.contains("counter_bump"), "what it adds is not shown: {asked:?}");
    assert!(asked.detail.contains("store.set(\"count\""), "the script is not shown: {asked:?}");
    owner.allows();

    assert_eq!(owner.hears().await, "It counts.");
    assert!(provider.last_offered().contains(&"counter_bump".to_owned()));
    assert_eq!(outcomes(&provider)[1..], ["Count: 1", "Count: 2"]);
}

#[tokio::test]
async fn the_owner_is_told_which_hosts_an_extension_would_reach() {
    let extensions = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    let mut asked = counter(COUNTER);
    asked["hosts"] = json!(["api.frankfurter.app"]);
    provider.then_acts([("create_extension", asked)]).then(Ok("Never mind.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Make yourself a counter.");
    let asked = owner.is_asked().await;
    assert!(asked.detail.contains("api.frankfurter.app"), "{asked:?}");
    owner.denies();
    owner.hears().await;

    assert!(
        !extensions.path().join("counter").exists(),
        "it was created without the Owner's approval"
    );
}

#[tokio::test]
async fn a_script_that_does_not_load_goes_back_to_the_model_not_to_the_owner() {
    let extensions = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([(
            "create_extension",
            counter("function describe() { return [{ name: 'bump' }]; }"),
        )])
        .then(Ok("I will fix it.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Make yourself a counter.");
    assert_eq!(owner.hears().await, "I will fix it.");

    assert!(owner.was_asked_nothing());
    let told = &outcomes(&provider)[0];
    assert!(told.contains("bump") && told.contains("description"), "{told}");
}

#[tokio::test]
async fn an_extension_cannot_take_the_name_of_one_that_ships_with_orbite() {
    let extensions = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    let mut asked = counter(COUNTER);
    asked["name"] = json!("tasks");
    provider.then_acts([("create_extension", asked)]).then(Ok("Another name then.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Replace the task list.");
    owner.hears().await;

    assert!(owner.was_asked_nothing());
    assert!(outcomes(&provider)[0].contains("tasks"), "{:?}", outcomes(&provider));
}

#[tokio::test]
async fn a_broken_extension_is_read_and_repaired_and_keeps_what_it_kept() {
    let extensions = tempfile::tempdir().unwrap();
    let broken = COUNTER.replace("return \"Count: \" + count;", "return \"Count: \" + cuont;");
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("create_extension", counter(&broken))])
        .then_acts([("counter_bump", json!({}))])
        .then_acts([("read_extension", json!({ "name": "counter" }))])
        .then_acts([("create_extension", counter(COUNTER))])
        .then_acts([("counter_bump", json!({}))])
        .then(Ok("Repaired.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Make yourself a counter.");
    owner.is_asked().await;
    owner.allows();
    let again = owner.is_asked().await;
    assert!(again.summary.contains("Replace"), "{again:?}");
    owner.allows();
    assert_eq!(owner.hears().await, "Repaired.");

    let told = outcomes(&provider);
    assert!(told[1].contains("cuont"), "the model was not told what failed: {}", told[1]);
    assert!(told[2].contains("cuont;"), "the model could not read its script: {}", told[2]);
    assert_eq!(told[4], "Count: 2", "what the Extension kept was lost in the repair");
}

#[tokio::test]
async fn repairing_an_extension_that_was_turned_off_turns_it_on_again() {
    let extensions = tempfile::tempdir().unwrap();
    fs::create_dir_all(extensions.path().join("counter")).unwrap();
    fs::write(
        extensions.path().join("counter/extension.json"),
        r#"{"description":"Counts things."}"#,
    )
    .unwrap();
    fs::write(extensions.path().join("counter/main.js"), "function describe( {").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("create_extension", counter(COUNTER))])
        .then_acts([("counter_bump", json!({}))])
        .then(Ok("Repaired.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Your counter is broken, repair it.");
    owner.is_asked().await;
    owner.allows();
    assert_eq!(owner.hears().await, "Repaired.");

    assert_eq!(outcomes(&provider).last().unwrap(), "Count: 1");
}

#[tokio::test]
async fn undoing_removes_the_extension_and_everything_it_kept() {
    let extensions = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("create_extension", counter(COUNTER))])
        .then_acts([("counter_bump", json!({}))])
        .then(Ok("Made.".into()))
        .then_acts([("remove_extension", json!({ "name": "counter" }))])
        .then(Ok("Gone.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);
    owner.says("Make yourself a counter.");
    owner.is_asked().await;
    owner.allows();
    owner.hears().await;
    assert!(extensions.path().join("counter/data.json").exists());

    owner.says("Undo that.");
    let asked = owner.is_asked().await;
    assert!(asked.irreversible && asked.detail.contains("counter"), "{asked:?}");
    owner.allows();
    assert_eq!(owner.hears().await, "Gone.");

    assert!(!extensions.path().join("counter").exists());
    assert!(!provider.last_offered().contains(&"counter_bump".to_owned()));
}

#[tokio::test]
async fn the_actions_of_an_extension_are_named_in_letters_every_model_accepts() {
    let extensions = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    let mut wanted = counter(COUNTER);
    wanted["name"] = json!("visit-counter");
    provider
        .then_acts([("create_extension", wanted)])
        .then_acts([("visit_counter_bump", json!({}))])
        .then(Ok("It counts.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Make yourself a visit counter.");
    assert!(owner.is_asked().await.detail.contains("visit_counter_bump"));
    owner.allows();
    owner.hears().await;

    assert_eq!(outcomes(&provider)[1], "Count: 1");
}
