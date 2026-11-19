//! What an Owner observes when they write to the Agent while it works.

mod support;

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use orbite::{Agent, Message, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};

fn agent_in(workspace: &Path, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let workspace = workspace.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .working_in(workspace)
            .stopping_commands_after(Duration::from_secs(20))
            .converse(&mut channel)
            .await
    });
    owner
}

async fn wait_for(path: &Path) {
    for _ in 0..300 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{} never appeared", path.display());
}

fn is_running(pid: &str) -> bool {
    // A stopped process stays listed as a zombie until it is reaped.
    fs::read_to_string(format!("/proc/{}/stat", pid.trim()))
        .is_ok_and(|stat| !stat.contains(") Z "))
}

#[tokio::test]
async fn stop_halts_a_running_command_and_everything_it_started() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([(
        "run_command",
        json!({ "command": "sleep 15 & echo $! > pid; wait; touch finished.txt" }),
    )]);
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Run the long job.");
    owner.is_asked().await;
    owner.allows();
    wait_for(&workspace.path().join("pid")).await;
    let pid = fs::read_to_string(workspace.path().join("pid")).unwrap();
    assert!(is_running(&pid));

    let asked_to_stop = Instant::now();
    owner.says("stop");

    assert_eq!(owner.hears().await, "Stopped.");
    assert!(asked_to_stop.elapsed() < Duration::from_secs(3));
    for _ in 0..100 {
        if !is_running(&pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!is_running(&pid), "what the command started is still running");
    assert!(!workspace.path().join("finished.txt").exists());
}

#[tokio::test]
async fn the_model_is_told_its_work_was_stopped() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([
            ("run_command", json!({ "command": "touch started; sleep 15" })),
            ("write_file", json!({ "path": "after.txt", "content": "x" })),
        ])
        .then(Ok("Understood.".into()));
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Run both.");
    owner.is_asked().await;
    owner.allows();
    wait_for(&workspace.path().join("started")).await;

    owner.says("/stop");
    assert_eq!(owner.hears().await, "Stopped.");
    owner.says("Why did you stop?");
    assert_eq!(owner.hears().await, "Understood.");

    let seen = provider.last_asked();
    let outcomes: Vec<&str> = seen
        .iter()
        .filter_map(|message| match message {
            Message::Outcome { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(outcomes.len(), 2, "every Action needs an Outcome: {seen:?}");
    assert!(outcomes.iter().all(|text| text.contains("stopped")), "{outcomes:?}");
    assert!(!workspace.path().join("after.txt").exists());
}

#[tokio::test]
async fn stop_also_halts_the_agent_while_it_thinks() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.then_never_answers().then(Ok("Hello.".into()));
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Think hard about this.");
    while provider.times_asked() == 0 {
        tokio::task::yield_now().await;
    }

    owner.says("Stop!");
    assert_eq!(owner.hears().await, "Stopped.");

    owner.says("Hi");
    assert_eq!(owner.hears().await, "Hello.");
}

#[tokio::test]
async fn what_the_owner_adds_while_the_agent_thinks_is_taken_into_account_at_once() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.then_never_answers().then(Ok("A red one, then.".into()));
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Find me a bike.");
    while provider.times_asked() == 0 {
        tokio::task::yield_now().await;
    }

    owner.says("A red one.");

    assert_eq!(owner.hears().await, "A red one, then.");
    assert_eq!(
        provider.last_asked(),
        [Message::from_owner("Find me a bike."), Message::from_owner("A red one.")]
    );
}

#[tokio::test]
async fn what_the_owner_writes_during_an_action_changes_what_the_agent_does_next() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([
            ("run_command", json!({ "command": "touch started; sleep 0.4; touch first.txt" })),
            ("write_file", json!({ "path": "second.txt", "content": "x" })),
        ])
        .then(Ok("I left the second file out.".into()));
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Make both files.");
    owner.is_asked().await;
    owner.allows();
    wait_for(&workspace.path().join("started")).await;

    owner.says("Actually, skip the second file.");

    assert_eq!(owner.hears().await, "I left the second file out.");
    assert!(workspace.path().join("first.txt").exists(), "an Action under way is finished");
    assert!(!workspace.path().join("second.txt").exists());
    assert!(owner.was_asked_nothing(), "the second Action must not even be asked about");
    let seen = provider.last_asked();
    assert_eq!(seen.last(), Some(&Message::from_owner("Actually, skip the second file.")));
    let Message::Outcome { text, .. } = &seen[seen.len() - 2] else { panic!("{seen:?}") };
    assert!(text.contains("not carried out"), "{text}");
}

#[tokio::test]
async fn stop_said_when_nothing_is_under_way_is_just_something_said() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["There is nothing to stop."]);
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("stop");

    assert_eq!(owner.hears().await, "There is nothing to stop.");
}

#[tokio::test]
async fn stop_written_instead_of_answering_an_approval_cancels_it() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([
            ("run_command", json!({ "command": "touch made.txt" })),
            ("write_file", json!({ "path": "after.txt", "content": "x" })),
        ])
        .then(Ok("Fine.".into()));
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Make them.");
    owner.is_asked().await;

    owner.says("stop");

    assert_eq!(owner.hears().await, "Stopped.");
    assert!(!workspace.path().join("made.txt").exists());
    assert!(!workspace.path().join("after.txt").exists());
    assert!(owner.was_asked_nothing(), "the second Action must not be asked about");
}

#[tokio::test]
async fn writing_instead_of_answering_an_approval_leaves_the_action_out_and_is_heard() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "touch made.txt" }))])
        .then(Ok("A different name, then.".into()));
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Make it.");
    owner.is_asked().await;

    owner.says("Wait, call it other.txt instead.");

    assert_eq!(owner.hears().await, "A different name, then.");
    assert!(!workspace.path().join("made.txt").exists());
    let seen = provider.last_asked();
    assert_eq!(seen.last(), Some(&Message::from_owner("Wait, call it other.txt instead.")));
    let Message::Outcome { text, .. } = &seen[seen.len() - 2] else { panic!("{seen:?}") };
    assert!(text.contains("not carried out"), "{text}");
}

#[tokio::test]
async fn reset_asked_while_the_agent_works_stops_it_and_starts_afresh() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.then_never_answers().then(Ok("Hello.".into()));
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Think hard about this.");
    while provider.times_asked() == 0 {
        tokio::task::yield_now().await;
    }

    owner.says("/reset");
    assert!(owner.hears().await.contains("afresh"));

    owner.says("Hi");
    assert_eq!(owner.hears().await, "Hello.");
    assert_eq!(provider.last_asked(), [Message::from_owner("Hi")]);
}

#[tokio::test]
async fn a_secret_or_a_sign_in_address_sent_while_the_agent_thinks_is_not_shown_to_the_model() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.then_never_answers().then(Ok("Noted.".into()));
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Think hard about this.");
    while provider.times_asked() == 0 {
        tokio::task::yield_now().await;
    }

    owner.says("http://127.0.0.1:1455/auth/callback?code=only-once&state=of-the-link");

    assert_eq!(owner.hears().await, "Noted.");
    let told = format!("{:?}", provider.last_asked());
    assert!(!told.contains("only-once") && !told.contains("of-the-link"), "{told}");
    assert!(told.contains("was not kept"), "{told}");
}
