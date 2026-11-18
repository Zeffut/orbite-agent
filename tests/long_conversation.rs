//! What an Owner observes of a Conversation that goes on for months.

mod support;

use std::path::Path;

use orbite::{Agent, Message, ProviderError, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};

/// A rough size in tokens, counted the way Orbite documents it: four
/// characters to a token.
fn size(instructions: &str, conversation: &[Message]) -> usize {
    let characters: usize = instructions.len()
        + conversation
            .iter()
            .map(|message| serde_json::to_string(message).unwrap().len())
            .sum::<usize>();
    characters / 4
}

fn is_summary_request(instructions: &str) -> bool {
    instructions.contains("summar")
}

/// A model that answers every message with a filler paragraph and writes
/// short summaries when asked for one.
fn talkative() -> ScriptedProvider {
    let provider = ScriptedProvider::default();
    provider.otherwise(|instructions, _| {
        Ok(if is_summary_request(instructions) {
            "SUMMARY: the Owner talked about many things.".into()
        } else {
            "A reasonably long answer that takes up some room in the Conversation. ".repeat(4)
        })
    });
    provider
}

fn agent_with(provider: &ScriptedProvider, store: Store, budget: usize) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    tokio::spawn(async move {
        Agent::new(provider, store).keeping_context_under(budget).converse(&mut channel).await
    });
    owner
}

async fn chat(owner: &mut Owner, messages: usize) {
    let messages = messages * 2;
    for number in 0..messages {
        owner.says(&format!(
            "Message number {number}, with a little padding to give it some weight."
        ));
        owner.hears().await;
    }
}

#[tokio::test]
async fn what_is_sent_each_turn_stays_within_budget_however_long_the_conversation() {
    let provider = talkative();
    let mut owner = agent_with(&provider, Store::in_memory(), 6_500);

    chat(&mut owner, 60).await;

    let calls = provider.every_call();
    let largest = calls
        .iter()
        .filter(|(instructions, _)| !is_summary_request(instructions))
        .map(|(instructions, conversation)| size(instructions, conversation))
        .max()
        .unwrap();
    assert!(largest <= 6_500, "a turn sent about {largest} tokens");
    let (_, latest) = calls.last().unwrap();
    assert!(latest.len() < 200, "the whole Conversation (240 entries) is still being sent");
}

#[tokio::test]
async fn what_was_left_out_is_summarised_for_the_model() {
    let provider = talkative();
    let mut owner = agent_with(&provider, Store::in_memory(), 6_500);

    chat(&mut owner, 60).await;

    assert!(
        provider.last_instructions().contains("SUMMARY: the Owner talked about many things."),
        "{}",
        provider.last_instructions()
    );
}

#[tokio::test]
async fn a_new_summary_builds_on_the_previous_one() {
    let provider = talkative();
    let mut owner = agent_with(&provider, Store::in_memory(), 6_500);

    chat(&mut owner, 60).await;

    let summaries: Vec<_> = provider
        .every_call()
        .into_iter()
        .filter(|(instructions, _)| is_summary_request(instructions))
        .collect();
    assert!(summaries.len() >= 2, "only {} summary was made", summaries.len());
    let (_, asked) = summaries.last().unwrap();
    let Some(Message::Owner(material)) = asked.first() else { panic!("{asked:?}") };
    assert!(material.contains("SUMMARY: the Owner talked about many things."), "{material}");
}

#[tokio::test]
async fn the_model_never_sees_an_action_without_what_came_of_it_or_the_reverse() {
    let workspace = tempfile::tempdir().unwrap();
    // Every Owner message leads to one command, then an answer.
    let scripted = ScriptedProvider::default();
    for _ in 0..40 {
        scripted
            .then_acts([("run_command", json!({ "command": "echo some output of a command" }))])
            .then(Ok("Done, and here is a sentence to take up room. ".repeat(3)));
    }
    scripted.otherwise(|instructions, _| {
        Ok(if is_summary_request(instructions) { "SUMMARY".into() } else { "ok".into() })
    });
    let (mut channel, mut owner) = memory_channel();
    let agent_provider = scripted.clone();
    let folder = workspace.path().to_owned();
    tokio::spawn(async move {
        Agent::new(agent_provider, Store::in_memory())
            .working_in(folder)
            .keeping_context_under(6_500)
            .converse(&mut channel)
            .await
    });

    for number in 0..40 {
        owner.says(&format!("Do thing number {number}."));
        owner.hears().await;
    }

    for (instructions, conversation) in scripted.every_call() {
        if is_summary_request(&instructions) {
            continue;
        }
        let mut open: Vec<String> = Vec::new();
        for message in &conversation {
            match message {
                Message::Agent { actions, .. } => open.extend(actions.iter().map(|a| a.id.clone())),
                Message::Outcome { action, .. } => {
                    assert!(open.contains(action), "an Outcome arrived without its Action");
                    open.retain(|id| id != action);
                }
                Message::Owner(_) | Message::Notice(_) => {}
            }
        }
        // Actions of the very last entry are the ones being carried out now.
        let pending_now = match conversation.last() {
            Some(Message::Agent { actions, .. }) => actions.len(),
            _ => 0,
        };
        assert_eq!(open.len(), pending_now, "an Action lost its Outcome: {conversation:?}");
    }
}

#[tokio::test]
async fn a_detail_from_long_ago_can_still_be_found() {
    let provider = talkative();
    let store_folder = tempfile::tempdir().unwrap();
    let mut owner =
        agent_with(&provider, Store::open(store_folder.path().join("orbite.db")).unwrap(), 6_500);
    owner.says("The door code of the cellar is 4417.");
    owner.hears().await;
    chat(&mut owner, 60).await;
    assert!(
        !serde_json::to_string(&provider.last_asked()).unwrap().contains("4417"),
        "the detail is still in what the model sees, so this proves nothing"
    );

    provider.then_acts([("search_conversation", json!({ "query": "cellar door code" }))]);
    owner.says("What was the cellar code again?");
    owner.hears().await;

    let calls = provider.every_call();
    let found = calls.iter().rev().find_map(|(_, conversation)| match conversation.last() {
        Some(Message::Outcome { text, .. }) => Some(text.clone()),
        _ => None,
    });
    assert!(found.as_deref().unwrap_or("").contains("4417"), "{found:?}");
}

#[tokio::test]
async fn searching_for_what_was_never_said_says_so() {
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("search_conversation", json!({ "query": "unicorn" }))])
        .then(Ok("No.".into()));
    let mut owner = agent_with(&provider, Store::in_memory(), 6_500);

    owner.says("Did we ever talk about it?");
    owner.hears().await;

    let Some(Message::Outcome { text, .. }) = provider.last_asked().pop() else { panic!() };
    assert!(text.contains("Nothing"), "{text}");
}

#[tokio::test]
async fn the_summary_survives_a_restart() {
    let home = tempfile::tempdir().unwrap();
    let database: &Path = &home.path().join("orbite.db");
    {
        let provider = talkative();
        let mut owner = agent_with(&provider, Store::open(database).unwrap(), 6_500);
        chat(&mut owner, 60).await;
    }

    let provider = talkative();
    let mut owner = agent_with(&provider, Store::open(database).unwrap(), 6_500);
    owner.says("Still there?");
    owner.hears().await;

    assert!(provider.last_instructions().contains("SUMMARY: the Owner talked about many things."));
    assert!(size(&provider.last_instructions(), &provider.last_asked()) <= 6_500);
}

#[tokio::test]
async fn when_summarising_fails_the_agent_still_answers_within_budget() {
    let provider = ScriptedProvider::default();
    provider.otherwise(|instructions, _| {
        if is_summary_request(instructions) {
            Err(ProviderError::Failed("overloaded".into()))
        } else {
            Ok("A reasonably long answer that takes up some room in the Conversation. ".repeat(4))
        }
    });
    let mut owner = agent_with(&provider, Store::in_memory(), 6_500);

    chat(&mut owner, 60).await;

    let largest = provider
        .every_call()
        .iter()
        .filter(|(instructions, _)| !is_summary_request(instructions))
        .map(|(instructions, conversation)| size(instructions, conversation))
        .max()
        .unwrap();
    assert!(largest <= 6_500, "a turn sent about {largest} tokens");
}

#[tokio::test]
async fn the_owner_can_start_afresh_without_losing_memory() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("memory")).unwrap();
    std::fs::write(home.path().join("memory/home.md"), "Lives in Lyon.\n").unwrap();
    let provider = ScriptedProvider::answering(["Noted.", "Hello again."]);
    let (mut channel, mut owner) = memory_channel();
    let agent_provider = provider.clone();
    let memory = home.path().join("memory");
    tokio::spawn(async move {
        Agent::new(agent_provider, Store::in_memory())
            .remembering_in(memory)
            .converse(&mut channel)
            .await
    });
    owner.says("My password hint is blue-horse.");
    owner.hears().await;

    owner.says("/reset");
    let confirmation = owner.hears().await;
    owner.says("Hi");
    owner.hears().await;

    assert!(confirmation.contains("afresh"), "{confirmation}");
    assert_eq!(provider.times_asked(), 2, "/reset is not sent to the model");
    assert_eq!(provider.last_asked(), [Message::from_owner("Hi")]);
    assert!(provider.last_instructions().contains("Lives in Lyon."));
    assert!(!provider.last_instructions().contains("blue-horse"));
}

#[tokio::test]
async fn one_enormous_outcome_does_not_push_a_turn_over_budget() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("big.txt"), "A line of a big file.\n".repeat(3_000))
        .unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("read_file", json!({ "path": "big.txt" }))]).then(Ok("It is big.".into()));
    let (mut channel, mut owner) = memory_channel();
    let agent_provider = provider.clone();
    let folder = workspace.path().to_owned();
    tokio::spawn(async move {
        Agent::new(agent_provider, Store::in_memory())
            .working_in(folder)
            .keeping_context_under(7_000)
            .converse(&mut channel)
            .await
    });

    owner.says("Read the big file.");
    owner.hears().await;

    let largest = provider
        .every_call()
        .iter()
        .map(|(instructions, conversation)| size(instructions, conversation))
        .max()
        .unwrap();
    assert!(largest <= 7_000, "a turn sent about {largest} tokens");
    let Some(Message::Outcome { text, .. }) = provider.last_asked().pop() else { panic!() };
    assert!(text.contains("cut"), "the model is told the Outcome is incomplete");
}
