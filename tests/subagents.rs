//! What an Owner observes when the Agent hands work to Sub-agents.

mod support;

use std::path::Path;

use orbite::{Agent, Message, ProviderError, Reply, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, acting, memory_channel, saying};

/// Sub-agents are told what they are; the Agent is not.
fn is_subagent(instructions: &str) -> bool {
    instructions.contains("sub-agent of")
}

fn agent_in(workspace: &Path, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let workspace = workspace.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .working_in(workspace)
            .giving_subagents_steps(6)
            .converse(&mut channel)
            .await
    });
    owner
}

/// An Agent that delegates whatever it is asked, tells its Owner so, and
/// relays the report when it comes; `subagent` plays the Sub-agent.
fn delegating(
    subagent: impl Fn(&[Message]) -> Result<Reply, ProviderError> + Send + 'static,
) -> ScriptedProvider {
    let provider = ScriptedProvider::default();
    provider.replying_otherwise(move |instructions, conversation| {
        if is_subagent(instructions) {
            return subagent(conversation);
        }
        Ok(match conversation.last() {
            Some(Message::Owner(text)) if text.contains("2+2") => saying("4"),
            Some(Message::Owner(_)) => {
                acting("delegate", json!({ "name": "counter", "task": "Count the files." }))
            }
            Some(Message::Notice(report)) => saying(&format!("Relayed: {report}")),
            _ => saying("A sub-agent is on it."),
        })
    });
    provider
}

#[tokio::test]
async fn work_handed_to_a_subagent_comes_back_as_a_report_the_agent_relays() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = delegating(|_| Ok(saying("There are 3 files.")));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Count my files.");

    assert_eq!(owner.hears().await, "A sub-agent is on it.");
    let relayed = owner.hears().await;
    assert!(relayed.contains("counter"), "{relayed}");
    assert!(relayed.contains("There are 3 files."), "{relayed}");
}

#[tokio::test]
async fn the_subagent_is_given_its_task_and_nothing_of_the_conversation() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = delegating(|_| Ok(saying("Done.")));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Count my files, and by the way my password is hunter2.");
    owner.hears().await;
    owner.hears().await;

    let (_, given) = provider
        .every_call()
        .into_iter()
        .find(|(instructions, _)| is_subagent(instructions))
        .unwrap();
    assert_eq!(given, [Message::from_owner("Count the files.")]);
}

#[tokio::test]
async fn the_owner_can_keep_talking_while_a_subagent_works() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = delegating(|_| Ok(saying("There are 3 files.")));
    let release = provider.holding("sub-agent of");
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Count my files.");
    assert_eq!(owner.hears().await, "A sub-agent is on it.");

    owner.says("Meanwhile, what is 2+2?");
    assert_eq!(owner.hears().await, "4");

    release.send(true).unwrap();
    assert!(owner.hears().await.contains("There are 3 files."));
}

#[tokio::test]
async fn running_subagents_can_be_listed_and_stopped() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.replying_otherwise(|instructions, conversation| {
        if is_subagent(instructions) {
            return Ok(saying("Finished anyway."));
        }
        Ok(match conversation.last() {
            Some(Message::Owner(text)) if text.contains("Start") => {
                acting("delegate", json!({ "name": "counter", "task": "Count the files." }))
            }
            Some(Message::Owner(text)) if text.contains("list") => {
                acting("list_subagents", json!({}))
            }
            Some(Message::Owner(_)) => acting("stop_subagent", json!({ "name": "counter" })),
            Some(Message::Outcome { text, .. }) => saying(text),
            other => panic!("unexpected: {other:?}"),
        })
    });
    let release = provider.holding("sub-agent of");
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says("Start counting.");
    owner.hears().await;

    owner.says("Please list them.");
    let listed = owner.hears().await;
    assert!(listed.contains("counter") && listed.contains("Count the files."), "{listed}");

    owner.says("Stop it.");
    assert!(owner.hears().await.contains("Stopped"));
    release.send(true).unwrap();
    assert!(owner.hears_nothing().await, "a stopped Sub-agent reports nothing");

    owner.says("Please list them.");
    assert!(owner.hears().await.contains("No sub-agent"));
}

#[tokio::test]
async fn a_subagent_can_look_but_cannot_do_what_needs_the_owners_approval() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("a.txt"), "x").unwrap();
    let provider = delegating(|conversation| {
        Ok(match conversation.len() {
            1 => acting("run_command", json!({ "command": "ls" })),
            3 => acting("write_file", json!({ "path": "made.txt", "content": "x" })),
            _ => {
                let outcomes: Vec<&str> = conversation
                    .iter()
                    .filter_map(|message| match message {
                        Message::Outcome { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                saying(&outcomes.join(" / "))
            }
        })
    });
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Count my files.");
    owner.hears().await;
    let relayed = owner.hears().await;

    assert!(relayed.contains("a.txt"), "looking is allowed: {relayed}");
    assert!(relayed.contains("approval"), "{relayed}");
    assert!(!workspace.path().join("made.txt").exists());
    assert!(owner.was_asked_nothing());
}

#[tokio::test]
async fn a_subagent_that_uses_up_its_steps_stops_and_still_reports() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = delegating(|conversation| {
        Ok(match conversation.last() {
            // Told it is out of steps, it reports what it has.
            Some(Message::Owner(text)) if text.contains("out of steps") => saying("Got half way."),
            _ => acting("run_command", json!({ "command": "pwd" })),
        })
    });
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Count my files.");
    owner.hears().await;
    let relayed = owner.hears().await;

    assert!(relayed.contains("Got half way."), "{relayed}");
    assert!(relayed.contains("ran out of steps"), "{relayed}");
    let subagent_calls =
        provider.every_call().iter().filter(|(instructions, _)| is_subagent(instructions)).count();
    assert_eq!(subagent_calls, 7, "six steps, then one call to report");
}

#[tokio::test]
async fn a_subagent_cannot_delegate_remember_or_change_skills() {
    let home = tempfile::tempdir().unwrap();
    let provider = delegating(|_| Ok(saying("Done.")));
    let (mut channel, mut owner) = memory_channel();
    let agent_provider = provider.clone();
    let folder = home.path().to_owned();
    tokio::spawn(async move {
        Agent::new(agent_provider, Store::in_memory())
            .working_in(folder.join("workspace"))
            .remembering_in(folder.join("memory"))
            .learning_in(folder.join("skills"))
            .converse(&mut channel)
            .await
    });

    owner.says("Count my files.");
    owner.hears().await;
    owner.hears().await;

    let (_, offered) = provider
        .every_offer()
        .into_iter()
        .find(|(instructions, _)| is_subagent(instructions))
        .unwrap();
    for own in ["delegate", "stop_subagent", "remember", "forget", "save_skill", "remove_skill"] {
        assert!(!offered.contains(&own.to_string()), "{own} was offered to a Sub-agent");
    }
    for shared in ["run_command", "read_file", "read_page", "search_web", "use_skill"] {
        assert!(offered.contains(&shared.to_string()), "{shared} was not offered to a Sub-agent");
    }
}

#[tokio::test]
async fn a_subagent_whose_provider_fails_is_reported_as_failed() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = delegating(|_| Err(ProviderError::Failed("model crashed".into())));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Count my files.");
    owner.hears().await;
    let relayed = owner.hears().await;

    assert!(relayed.contains("failed") && relayed.contains("model crashed"), "{relayed}");
}

#[tokio::test]
async fn two_subagents_cannot_share_a_name() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.replying_otherwise(|instructions, conversation| {
        if is_subagent(instructions) {
            return Ok(saying("Done."));
        }
        Ok(match conversation.last() {
            Some(Message::Owner(_)) => {
                acting("delegate", json!({ "name": "counter", "task": "Count the files." }))
            }
            Some(Message::Outcome { text, .. }) => saying(text),
            other => panic!("unexpected: {other:?}"),
        })
    });
    let _release = provider.holding("sub-agent of");
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Count.");
    owner.hears().await;
    owner.says("Count again.");

    assert!(owner.hears().await.contains("already"));
}
