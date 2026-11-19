mod support;

use std::time::Duration;

use orbite::{Agent, Message, ProviderError, Store};
use support::{ScriptedProvider, memory_channel};

#[tokio::test]
async fn the_owner_gets_the_agents_answer() {
    let provider = ScriptedProvider::answering(["Hello, I'm here."]);
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(
        async move { Agent::new(provider, Store::in_memory()).converse(&mut channel).await },
    );

    owner.says("Hi");

    assert_eq!(owner.hears().await, "Hello, I'm here.");
}

#[tokio::test]
async fn the_agent_keeps_the_thread_of_the_conversation() {
    let provider = ScriptedProvider::answering(["Noted.", "Lyon."]);
    let (mut channel, mut owner) = memory_channel();
    let agent_provider = provider.clone();
    tokio::spawn(async move {
        Agent::new(agent_provider, Store::in_memory()).converse(&mut channel).await
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
    provider.then(Err(ProviderError::Rejected("invalid api key".into()))).then(Ok("Back.".into()));
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(
        async move { Agent::new(provider, Store::in_memory()).converse(&mut channel).await },
    );

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
    tokio::spawn(
        async move { Agent::new(provider, Store::in_memory()).converse(&mut channel).await },
    );

    owner.says("Hi");

    let explanation = owner.hears().await;
    assert!(explanation.contains("couldn't reach"), "{explanation}");
}

#[tokio::test]
async fn a_busy_provider_is_waited_for_without_troubling_the_owner() {
    let provider = ScriptedProvider::default();
    let busy = || ProviderError::Busy {
        reason: "rate limit".into(),
        retry_after: Some(Duration::from_millis(10)),
    };
    provider.then(Err(busy())).then(Err(busy())).then(Ok("Here I am.".into()));
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(
        async move { Agent::new(provider, Store::in_memory()).converse(&mut channel).await },
    );

    owner.says("Hi");

    assert_eq!(owner.hears().await, "Here I am.");
}

#[tokio::test]
async fn a_provider_that_stays_busy_is_explained_and_the_message_can_be_sent_again() {
    let provider = ScriptedProvider::default();
    for _ in 0..20 {
        provider.then(Err(ProviderError::Busy {
            reason: "rate limit".into(),
            retry_after: Some(Duration::from_millis(5)),
        }));
    }
    let asked = provider.clone();
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .waiting_for_a_busy_provider(Duration::from_millis(1))
            .converse(&mut channel)
            .await
    });

    owner.says("Hi");

    let explanation = owner.hears().await;
    assert!(explanation.contains("busy"), "{explanation}");
    assert!(asked.times_asked() < 20, "the Agent must give up at some point");
}

#[tokio::test]
async fn a_provider_asking_for_a_long_wait_is_not_waited_for() {
    let provider = ScriptedProvider::default();
    provider.then(Err(ProviderError::Busy {
        reason: "daily limit reached".into(),
        retry_after: Some(Duration::from_secs(3600)),
    }));
    let asked = provider.clone();
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(
        async move { Agent::new(provider, Store::in_memory()).converse(&mut channel).await },
    );

    owner.says("Hi");

    let explanation = owner.hears().await;
    assert!(explanation.contains("daily limit reached"), "{explanation}");
    assert_eq!(asked.times_asked(), 1);
}

#[tokio::test]
async fn the_agent_knows_the_name_its_owner_gave_it() {
    let provider = ScriptedProvider::answering(["I'm Nova."]);
    let asked = provider.clone();
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory()).called("Nova").converse(&mut channel).await
    });

    owner.says("Who are you?");
    owner.hears().await;

    assert!(
        asked.last_instructions().contains("Your name is Nova"),
        "{}",
        asked.last_instructions()
    );
}
