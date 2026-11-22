//! What every turn costs before anything is said: the instructions and the
//! Actions on offer are sent to the model each time, and paid each time.

mod support;

use std::sync::{Arc, Mutex};

use orbite::{Agent, Provider, ProviderError, Reply, Request, Store};
use support::memory_channel;

/// Notes how much it is sent, in tokens at four characters each.
#[derive(Clone, Default)]
struct Weighing(Arc<Mutex<Vec<(String, usize)>>>);

impl Provider for Weighing {
    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        let mut weighed = vec![("(instructions)".to_owned(), request.instructions.len() / 4)];
        for action in request.actions {
            let size =
                action.name.len() + action.description.len() + action.input.to_string().len();
            weighed.push((action.name.clone(), size / 4));
        }
        *self.0.lock().unwrap() = weighed;
        Ok(Reply { text: "Hi.".into(), actions: Vec::new(), kept: None, usage: None })
    }
}

#[tokio::test]
async fn a_turn_with_everything_on_offer_stays_light() {
    let home = tempfile::tempdir().unwrap();
    let provider = Weighing::default();
    let (mut channel, mut owner) = memory_channel();
    let (model, folder) = (provider.clone(), home.path().to_owned());
    tokio::spawn(async move {
        Agent::new(model, Store::in_memory())
            .working_in(folder.join("workspace"))
            .remembering_in(folder.join("memory"))
            .learning_in(folder.join("skills"))
            .extending_in(folder.join("extensions"))
            .keeping_secrets_in(&folder)
            .using_a_computer_in(folder.join("computer"))
            .converse(&mut channel)
            .await
    });
    owner.says("Hi");
    owner.hears().await;

    let mut weighed = provider.0.lock().unwrap().clone();
    let total: usize = weighed.iter().map(|(_, size)| size).sum();
    weighed.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
    let listed: Vec<String> =
        weighed.iter().map(|(name, size)| format!("{size:>5} {name}")).collect();
    // It was 4,009 with each Action offered on its own, then 3,200; some 220
    // more let the Agent do in plain words what took a command to write, and
    // fifteen tell it which Computer it is on: left to guess, it guessed wrong.
    assert!(total <= 3_470, "{total} tokens before the Conversation:\n{}", listed.join("\n"));
}
