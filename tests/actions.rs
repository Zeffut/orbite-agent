//! What an Owner observes when the Agent acts on the Host.

mod support;

use std::fs;
use std::path::Path;
use std::time::Duration;

use orbite::{Agent, Message, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};

/// Starts an Agent working in `workspace`, thinking with `provider`.
fn agent_in(workspace: &Path, provider: &ScriptedProvider) -> Owner {
    // Long enough for a quick command on a machine busy with other things.
    agent_stopping_commands_after(Duration::from_secs(20), workspace, provider)
}

fn agent_stopping_commands_after(
    patience: Duration,
    workspace: &Path,
    provider: &ScriptedProvider,
) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let workspace = workspace.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .working_in(workspace)
            .stopping_commands_after(patience)
            .converse(&mut channel)
            .await
    });
    owner
}

/// What the model was told came of its latest Action.
fn outcome(provider: &ScriptedProvider) -> String {
    match provider.last_asked().pop() {
        Some(Message::Outcome { text, .. }) => text,
        other => panic!("the model was not given an Outcome but {other:?}"),
    }
}

#[tokio::test]
async fn the_agent_inspects_the_host_without_asking() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "echo hello | wc -c" }))])
        .then(Ok("Six characters.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("How long is hello?");

    assert_eq!(owner.hears().await, "Six characters.");
    assert!(outcome(&provider).contains('6'), "{}", outcome(&provider));
    assert!(owner.was_asked_nothing());
}

#[tokio::test]
async fn commands_run_in_the_agents_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("notes.txt"), "remember the milk").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "cat notes.txt" }))])
        .then(Ok("Done.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Read my notes.");
    owner.hears().await;

    assert!(outcome(&provider).contains("remember the milk"), "{}", outcome(&provider));
}

#[tokio::test]
async fn a_command_that_changes_something_waits_for_an_approval() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "touch made.txt" }))])
        .then(Ok("Created.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Create made.txt");
    let approval = owner.is_asked().await;

    assert!(approval.detail.contains("touch made.txt"), "{approval:?}");
    assert!(approval.irreversible, "Orbite cannot undo a command: {approval:?}");
    assert!(!workspace.path().join("made.txt").exists(), "ran before the Approval");

    owner.allows();
    assert_eq!(owner.hears().await, "Created.");
    assert!(workspace.path().join("made.txt").exists());
}

#[tokio::test]
async fn a_denied_action_is_not_run_and_the_agent_is_told() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "touch made.txt" }))])
        .then(Ok("Understood, I left it alone.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Create made.txt");
    owner.is_asked().await;
    owner.denies();

    assert_eq!(owner.hears().await, "Understood, I left it alone.");
    assert!(!workspace.path().join("made.txt").exists());
    assert!(outcome(&provider).contains("denied"), "{}", outcome(&provider));
}

#[tokio::test]
async fn commands_that_could_change_something_are_never_taken_for_inspection() {
    for command in [
        "rm notes.txt",
        "ls; rm notes.txt",
        "ls && rm notes.txt",
        "echo hi > notes.txt",
        "cat $(rm notes.txt)",
        "cat `rm notes.txt`",
        "find . -delete",
        "find . -exec rm {} +",
        "ls\nrm notes.txt",
        "sudo ls",
        "curl https://example.org",
        "cat ../s*.key",
        "cat ~/.s?h/id_e*",
        "cat [n]otes.txt ../secrets.sealed",
        "grep -z TOKEN /proc/self/environ",
    ] {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("notes.txt"), "keep me").unwrap();
        let provider = ScriptedProvider::default();
        provider.then_acts([("run_command", json!({ "command": command }))]).then(Ok("Ok.".into()));
        let mut owner = agent_in(workspace.path(), &provider);

        owner.says("Go.");
        owner.is_asked().await;
        owner.denies();
        owner.hears().await;

        assert!(workspace.path().join("notes.txt").exists(), "{command:?} ran unasked");
    }
}

#[tokio::test]
async fn reading_what_looks_like_a_secret_waits_for_an_approval() {
    let workspace = tempfile::tempdir().unwrap();
    for (action, input) in [
        ("run_command", json!({ "command": "cat ~/.ssh/id_ed25519" })),
        ("read_file", json!({ "path": "/home/someone/.config/app/secrets.env" })),
    ] {
        let provider = ScriptedProvider::default();
        provider.then_acts([(action, input)]).then(Ok("Ok.".into()));
        let mut owner = agent_in(workspace.path(), &provider);

        owner.says("Go.");
        let approval = owner.is_asked().await;
        owner.denies();
        owner.hears().await;

        assert!(approval.detail.contains("secret"), "{approval:?}");
    }
}

#[tokio::test]
async fn a_command_that_hangs_is_stopped_and_the_conversation_goes_on() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "echo started; sleep 30" }))])
        .then(Ok("It hung.".into()));
    let mut owner =
        agent_stopping_commands_after(Duration::from_millis(400), workspace.path(), &provider);

    owner.says("Run it.");
    owner.is_asked().await;
    owner.allows();

    assert_eq!(owner.hears().await, "It hung.");
    let outcome = outcome(&provider);
    assert!(outcome.contains("stopped"), "{outcome}");
    assert!(outcome.contains("started"), "what it printed first is kept: {outcome}");
}

#[tokio::test]
async fn a_command_that_floods_output_does_not_flood_the_model() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "yes flood" }))])
        .then(Ok("Noisy.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Run it.");
    owner.is_asked().await;
    owner.allows();

    assert_eq!(owner.hears().await, "Noisy.");
    let outcome = outcome(&provider);
    assert!(outcome.len() < 40_000, "{} bytes reached the model", outcome.len());
    assert!(outcome.contains("cut"), "the model is told output is missing");
}

#[tokio::test]
async fn a_failing_command_reports_its_error_and_exit_code() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "cat missing.txt" }))])
        .then(Ok("No such file.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Read it.");
    owner.hears().await;

    let outcome = outcome(&provider);
    assert!(outcome.contains("No such file"), "{outcome}");
    assert!(outcome.contains("exit code 1"), "{outcome}");
}

#[tokio::test]
async fn the_agent_reads_a_file_without_asking() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("notes.txt"), "remember the milk").unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("read_file", json!({ "path": "notes.txt" }))]).then(Ok("Milk.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("What's in my notes?");

    assert_eq!(owner.hears().await, "Milk.");
    assert_eq!(outcome(&provider), "remember the milk");
    assert!(owner.was_asked_nothing());
}

#[tokio::test]
async fn creating_a_file_asks_and_says_it_can_be_undone() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("write_file", json!({ "path": "sub/list.txt", "content": "milk\n" }))])
        .then(Ok("Saved.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Save my list.");
    let approval = owner.is_asked().await;
    assert!(!approval.irreversible, "{approval:?}");
    owner.allows();

    assert_eq!(owner.hears().await, "Saved.");
    assert_eq!(fs::read_to_string(workspace.path().join("sub/list.txt")).unwrap(), "milk\n");
}

#[tokio::test]
async fn replacing_a_file_says_plainly_that_it_cannot_be_undone() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("list.txt"), "old").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("write_file", json!({ "path": "list.txt", "content": "new" }))])
        .then(Ok("Left as it was.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Replace my list.");
    let approval = owner.is_asked().await;
    owner.denies();
    owner.hears().await;

    assert!(approval.irreversible, "{approval:?}");
    assert_eq!(fs::read_to_string(workspace.path().join("list.txt")).unwrap(), "old");
}

#[tokio::test]
async fn editing_a_file_changes_only_the_passage_named() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("list.txt"), "milk\neggs\nbread\n").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("edit_file", json!({ "path": "list.txt", "old": "eggs", "new": "six eggs" }))])
        .then(Ok("Edited.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Make it six eggs.");
    let approval = owner.is_asked().await;
    assert!(
        approval.detail.contains("eggs") && approval.detail.contains("six eggs"),
        "{approval:?}"
    );
    owner.allows();
    owner.hears().await;

    assert_eq!(
        fs::read_to_string(workspace.path().join("list.txt")).unwrap(),
        "milk\nsix eggs\nbread\n"
    );
}

#[tokio::test]
async fn an_edit_whose_passage_is_missing_or_ambiguous_changes_nothing() {
    for old in ["butter", "e"] {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("list.txt"), "milk\neggs\nbread\n").unwrap();
        let provider = ScriptedProvider::default();
        provider
            .then_acts([("edit_file", json!({ "path": "list.txt", "old": old, "new": "x" }))])
            .then(Ok("Could not.".into()));
        let mut owner = agent_in(workspace.path(), &provider);

        owner.says("Edit.");
        assert_eq!(owner.hears().await, "Could not.");

        assert!(owner.was_asked_nothing(), "nothing to approve when the edit cannot apply");
        assert_eq!(
            fs::read_to_string(workspace.path().join("list.txt")).unwrap(),
            "milk\neggs\nbread\n"
        );
        assert!(outcome(&provider).contains("list.txt"), "{}", outcome(&provider));
    }
}

#[tokio::test]
async fn an_action_the_agent_does_not_have_or_cannot_understand_is_explained_to_the_model() {
    let workspace = tempfile::tempdir().unwrap();
    for (action, input, expected) in [
        ("launch_rocket", json!({}), "launch_rocket"),
        ("run_command", json!({ "cmd": "ls" }), "command"),
    ] {
        let provider = ScriptedProvider::default();
        provider.then_acts([(action, input)]).then(Ok("Sorry.".into()));
        let mut owner = agent_in(workspace.path(), &provider);

        owner.says("Go.");
        owner.hears().await;

        assert!(outcome(&provider).contains(expected), "{}", outcome(&provider));
    }
}

#[tokio::test]
async fn what_the_agent_says_while_acting_reaches_the_owner() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_says_and_acts("Let me look.", [("run_command", json!({ "command": "pwd" }))])
        .then(Ok("You are in your workspace.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Where are we?");

    assert_eq!(owner.hears().await, "Let me look.");
    assert_eq!(owner.hears().await, "You are in your workspace.");
}

#[tokio::test]
async fn an_action_cut_short_by_a_stop_is_reported_to_the_model_at_the_next_start() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("orbite.db");

    let provider = ScriptedProvider::default();
    provider.then_acts([("run_command", json!({ "command": "touch made.txt" }))]);
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(&database).unwrap();
    let running =
        tokio::spawn(async move { Agent::new(provider, store).converse(&mut channel).await });
    owner.says("Create made.txt");
    owner.is_asked().await;
    running.abort();
    let _ = running.await;

    let provider = ScriptedProvider::answering(["I was interrupted; shall I try again?"]);
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(&database).unwrap();
    let agent_provider = provider.clone();
    tokio::spawn(async move { Agent::new(agent_provider, store).converse(&mut channel).await });

    assert_eq!(owner.hears().await, "I was interrupted; shall I try again?");
    assert!(outcome(&provider).contains("interrupted"), "{}", outcome(&provider));
}

#[tokio::test]
async fn an_agent_that_never_stops_acting_is_stopped() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    for _ in 0..200 {
        provider.then_acts([("run_command", json!({ "command": "pwd" }))]);
    }
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Loop.");

    assert!(owner.hears().await.contains("stopped"), "the Owner is told");
    assert!(provider.times_asked() < 200);
}

#[tokio::test]
async fn editing_what_looks_like_a_secret_is_asked_before_the_file_is_looked_at() {
    let workspace = tempfile::tempdir().unwrap();
    fs::create_dir_all(workspace.path().join(".ssh")).unwrap();
    fs::write(workspace.path().join(".ssh/id_rsa"), "PRIVATE-KEY-MATERIAL").unwrap();
    let provider = ScriptedProvider::default();
    // A passage that is not there: saying so would tell what the file holds.
    provider
        .then_acts([("edit_file", json!({ "path": ".ssh/id_rsa", "old": "GUESS", "new": "x" }))])
        .then(Ok("Left alone.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Edit my key.");
    let asked = owner.is_asked().await;
    assert!(asked.critical, "{asked:?}");
    owner.denies();
    owner.hears().await;

    assert_eq!(
        fs::read_to_string(workspace.path().join(".ssh/id_rsa")).unwrap(),
        "PRIVATE-KEY-MATERIAL"
    );
}
