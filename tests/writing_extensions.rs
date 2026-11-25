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

#[tokio::test]
async fn the_owner_is_told_which_secrets_an_extension_would_use() {
    let extensions = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    let mut wanted = counter(COUNTER);
    wanted["hosts"] = json!(["api.example.org"]);
    wanted["secrets"] = json!(["WEATHER_KEY"]);
    provider.then_acts([("create_extension", wanted)]).then(Ok("Made.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Make yourself a weather connector.");
    let asked = owner.is_asked().await;
    assert!(asked.detail.contains("WEATHER_KEY"), "{asked:?}");
    owner.allows();
    owner.hears().await;

    let kept = fs::read_to_string(extensions.path().join("counter/extension.json")).unwrap();
    assert!(kept.contains("WEATHER_KEY"), "{kept}");
}

#[tokio::test]
async fn a_script_the_owner_has_not_approved_changes_nothing_that_is_kept() {
    let extensions = tempfile::tempdir().unwrap();
    // A counter that has counted, then a script of the same name that wipes
    // what is kept, as soon as it is loaded.
    let wiping = format!(
        "store.keys().forEach(key => store.remove(key)); store.set('planted', 1);\n{COUNTER}"
    );
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("create_extension", counter(COUNTER))])
        .then_acts([("counter_bump", json!({}))])
        .then_acts([("create_extension", counter(&wiping))])
        .then_acts([("counter_bump", json!({}))])
        .then(Ok("Done.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Make a counter, then change it.");
    owner.is_asked().await;
    owner.allows();
    owner.is_asked().await;
    owner.denies();
    owner.hears().await;

    assert_eq!(
        outcomes(&provider).last().unwrap(),
        "Count: 2",
        "what the counter kept was changed by a script nobody approved"
    );
    let kept = fs::read_to_string(extensions.path().join("counter/data.json")).unwrap();
    assert!(!kept.contains("planted"), "{kept}");
}

#[tokio::test]
async fn writing_straight_into_orbites_own_folders_is_asked_about_below_the_free_level() {
    let home = tempfile::tempdir().unwrap();
    let (mut channel, mut owner) = memory_channel();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("set_permission_level", json!({ "level": "balanced" }))])
        .then_acts([("write_file", json!({ "path": "../extensions/sneaky/main.js", "content": "function describe() { return []; }" }))])
        .then_acts([("write_file", json!({ "path": "../mcp.json", "content": "{}" }))])
        .then_acts([("write_file", json!({ "path": "notes.txt", "content": "fine" }))])
        .then(Ok("Done.".into()));
    let (model, folder) = (provider.clone(), home.path().to_owned());
    tokio::spawn(async move {
        Agent::new(model, Store::in_memory())
            .working_in(folder.join("workspace"))
            .extending_in(folder.join("extensions"))
            .connecting_servers_in(&folder)
            .converse(&mut channel)
            .await
    });

    owner.says("Be freer, then write these.");
    owner.is_asked().await; // loosening the level
    owner.allows();
    // Free within its own folders, and still asked: these files decide
    // what the Agent can reach.
    let first = owner.is_asked().await;
    assert!(first.critical && first.detail.contains("extensions/sneaky"), "{first:?}");
    owner.denies();
    let second = owner.is_asked().await;
    assert!(second.critical && second.detail.contains("mcp.json"), "{second:?}");
    owner.denies();
    owner.hears().await;

    assert!(owner.was_asked_nothing(), "an ordinary file of its own asked at the balanced level");
    assert!(
        !home.path().join("extensions/sneaky").exists() && !home.path().join("mcp.json").exists()
    );
}

#[tokio::test]
async fn an_extension_is_named_so_that_its_actions_are_its_own_and_every_provider_takes_them() {
    let extensions = tempfile::tempdir().unwrap();
    let named = |name: &str, script: &str| {
        let mut wanted = counter(script);
        wanted["name"] = json!(name);
        wanted
    };
    // An Extension "visit" with an Action "counter_bump", then one "visit-counter" with "bump".
    let clashing = COUNTER.replace("name: \"bump\"", "name: \"counter_bump\"");
    let long = COUNTER.replace("name: \"bump\"", &format!("name: \"{}\"", "b".repeat(70)));
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("create_extension", named("météo", COUNTER))])
        .then_acts([("create_extension", named("long", &long))])
        .then_acts([("create_extension", named("visit", &clashing))])
        .then_acts([("create_extension", named("visit-counter", COUNTER))])
        .then(Ok("Done.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Make these.");
    // Only the third is sound, and so the only one the Owner is asked about.
    let asked = owner.is_asked().await;
    assert!(asked.detail.contains("visit_counter_bump"), "{asked:?}");
    owner.allows();
    owner.hears().await;

    assert!(owner.was_asked_nothing());
    let told = outcomes(&provider);
    assert!(told[0].contains("cannot name an Extension"), "{}", told[0]);
    assert!(told[1].contains("too long"), "{}", told[1]);
    assert!(told[3].contains("already adds an Action called visit_counter_bump"), "{}", told[3]);
    assert!(!extensions.path().join("visit-counter").exists());
}

/// An Extension that says of itself whether it works.
const CHECKED: &str = r#"
function describe() {
  return [{ name: "bump", description: "Counts one more.", parameters: {} }];
}
function run() { return "Count: 1"; }
function check() { return "counted to 1"; }
"#;

#[tokio::test]
async fn what_the_agent_writes_is_tried_the_moment_it_is_in_place() {
    let extensions = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("create_extension", counter(CHECKED))]).then(Ok("It works.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Write yourself a counter.");
    owner.is_asked().await;
    owner.allows();
    owner.hears().await;

    let told = outcomes(&provider).pop().unwrap();
    assert!(told.contains("Its check passed: counted to 1"), "{told}");
}

#[tokio::test]
async fn what_does_not_work_is_said_so_and_sent_back_to_be_repaired() {
    let extensions = tempfile::tempdir().unwrap();
    let broken = CHECKED
        .replace(r#"return "counted to 1";"#, r#"throw new Error("the service answered 401");"#);
    let provider = ScriptedProvider::default();
    provider.then_acts([("create_extension", counter(&broken))]).then(Ok("Not yet.".into()));
    let mut owner = agent_extended_in(extensions.path(), &provider);

    owner.says("Write yourself a counter.");
    owner.is_asked().await;
    owner.allows();
    owner.hears().await;

    let told = outcomes(&provider).pop().unwrap();
    assert!(told.contains("check FAILED: the service answered 401"), "{told}");
    assert!(told.contains("Do not tell your Owner it works"), "{told}");
}
