mod support;

use orbite::{Agent, Message, ProviderError, Store};
use support::{ScriptedProvider, memory_channel};

#[tokio::test]
async fn the_owner_gets_the_agents_answer() {
    let provider = ScriptedProvider::answering(["Hello, I'm here."]);
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .converse(&mut channel)
            .await
    });

    owner.says("Hi");

    assert_eq!(owner.hears().await, "Hello, I'm here.");
}

#[tokio::test]
async fn the_agent_keeps_the_thread_of_the_conversation() {
    let provider = ScriptedProvider::answering(["Noted.", "Lyon."]);
    let (mut channel, mut owner) = memory_channel();
    let agent_provider = provider.clone();
    tokio::spawn(async move {
        Agent::new(agent_provider, Store::in_memory())
            .converse(&mut channel)
            .await
    });

    owner.says("I live in Lyon.");
    owner.hears().await;
    owner.says("Where do I live?");
    owner.hears().await;

    assert_eq!(
        provider.last_asked(),
        [
            Message::from_owner("I live in Lyon."),
            Message::from_agent("Noted."),
            Message::from_owner("Where do I live?"),
        ]
    );
}

#[tokio::test]
async fn a_rejected_api_key_is_explained_and_the_conversation_goes_on() {
    let provider = ScriptedProvider::default();
    provider
        .then(Err(ProviderError::Rejected("invalid api key".into())))
        .then(Ok("Back.".into()));
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .converse(&mut channel)
            .await
    });

    owner.says("Hi");
    let explanation = owner.hears().await;
    assert!(explanation.contains("refused my access"), "{explanation}");
    assert!(explanation.contains("invalid api key"), "{explanation}");

    owner.says("And now?");
    assert_eq!(owner.hears().await, "Back.");
}

#[tokio::test]
async fn an_unreachable_provider_is_explained() {
    let provider = ScriptedProvider::default();
    provider.then(Err(ProviderError::Unreachable("connection refused".into())));
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .converse(&mut channel)
            .await
    });

    owner.says("Hi");

    let explanation = owner.hears().await;
    assert!(explanation.contains("couldn't reach"), "{explanation}");
}
