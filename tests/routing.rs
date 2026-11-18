//! Several Providers behind the Agent: falling back when one fails, and
//! lighter work going to a lighter model.

mod support;

use std::time::Duration;

use orbite::{Agent, Message, Provider, ProviderError, Request, Routed, Store, Usage};
use support::{ScriptedProvider, memory_channel};

fn hi(light: bool) -> Request<'static> {
    static HI: std::sync::LazyLock<Vec<Message>> =
        std::sync::LazyLock::new(|| vec![Message::from_owner("Hi")]);
    Request { instructions: "", conversation: &HI, actions: &[], images: &[], light }
}

fn down() -> ProviderError {
    ProviderError::Unreachable("connection refused".into())
}

fn by(usage: Option<Usage>) -> String {
    usage.expect("who answered must be said").by
}

#[tokio::test]
async fn the_first_provider_answers_when_it_can() {
    let first = ScriptedProvider::answering(["From the first."]);
    let second = ScriptedProvider::default();
    let routed = Routed::through("first", first).or("second", second.clone());

    let reply = routed.reply(hi(false)).await.unwrap();

    assert_eq!(reply.text, "From the first.");
    assert_eq!(by(reply.usage), "first");
    assert_eq!(second.times_asked(), 0);
}

#[tokio::test]
async fn the_next_provider_answers_when_one_is_down_busy_or_refuses_the_key() {
    for failure in [
        down(),
        ProviderError::Busy { reason: "rate limit".into(), retry_after: None },
        ProviderError::Rejected("invalid key".into()),
    ] {
        let first = ScriptedProvider::default();
        first.then(Err(failure));
        let second = ScriptedProvider::answering(["From the second."]);
        let routed = Routed::through("first", first).or("second", second);

        let reply = routed.reply(hi(false)).await.unwrap();

        assert_eq!(reply.text, "From the second.");
        assert_eq!(by(reply.usage), "second");
    }
}

#[tokio::test]
async fn a_request_the_model_declines_or_cannot_take_is_not_tried_elsewhere() {
    for failure in [
        ProviderError::Declined("cyber".into()),
        ProviderError::Failed("HTTP 400: bad request".into()),
    ] {
        let first = ScriptedProvider::default();
        first.then(Err(failure.clone()));
        let second = ScriptedProvider::answering(["From the second."]);
        let routed = Routed::through("first", first).or("second", second.clone());

        assert_eq!(routed.reply(hi(false)).await.unwrap_err(), failure);
        assert_eq!(second.times_asked(), 0);
    }
}

#[tokio::test]
async fn when_every_provider_fails_the_first_ones_failure_is_reported() {
    let first = ScriptedProvider::default();
    first.then(Err(ProviderError::Rejected("invalid key".into())));
    let second = ScriptedProvider::default();
    second.then(Err(down()));
    let routed = Routed::through("first", first).or("second", second);

    assert_eq!(
        routed.reply(hi(false)).await.unwrap_err(),
        ProviderError::Rejected("invalid key".into())
    );
}

#[tokio::test]
async fn a_provider_that_failed_is_left_alone_for_a_while_then_tried_again() {
    let first = ScriptedProvider::default();
    first.then(Err(down())).then(Ok("Back.".into()));
    let second = ScriptedProvider::answering(["Second, once.", "Second, twice."]);
    let routed = Routed::through("first", first.clone())
        .or("second", second)
        .resting_failed_providers_for(Duration::from_millis(80));

    assert_eq!(routed.reply(hi(false)).await.unwrap().text, "Second, once.");
    assert_eq!(routed.reply(hi(false)).await.unwrap().text, "Second, twice.");
    assert_eq!(first.times_asked(), 1, "a Provider that is down is not asked at every turn");

    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(routed.reply(hi(false)).await.unwrap().text, "Back.");
}

#[tokio::test]
async fn light_work_goes_to_the_light_model_and_the_rest_does_not() {
    let heavy = ScriptedProvider::answering(["Heavy."]);
    let light = ScriptedProvider::answering(["Light."]);
    let routed = Routed::through("heavy", heavy).with_light("light", light);

    let lightly = routed.reply(hi(true)).await.unwrap();
    let heavily = routed.reply(hi(false)).await.unwrap();

    assert_eq!((lightly.text.as_str(), by(lightly.usage).as_str()), ("Light.", "light"));
    assert_eq!((heavily.text.as_str(), by(heavily.usage).as_str()), ("Heavy.", "heavy"));
}

#[tokio::test]
async fn light_work_falls_back_on_the_other_models_and_is_plain_work_without_a_light_one() {
    let heavy = ScriptedProvider::answering(["Heavy does it."]);
    let light = ScriptedProvider::default();
    light.then(Err(down()));
    let routed = Routed::through("heavy", heavy).with_light("light", light);
    assert_eq!(routed.reply(hi(true)).await.unwrap().text, "Heavy does it.");

    let only = ScriptedProvider::answering(["The only one."]);
    let routed = Routed::through("only", only);
    assert_eq!(routed.reply(hi(true)).await.unwrap().text, "The only one.");
}

// --- As the Owner sees it ---

#[tokio::test]
async fn the_owner_is_told_once_when_another_provider_takes_over_and_when_the_usual_one_is_back() {
    let first = ScriptedProvider::default();
    first.then(Err(down())).then(Ok("Three.".into()));
    let second = ScriptedProvider::answering(["One.", "Two."]);
    let routed = Routed::through("first", first)
        .or("second", second)
        .resting_failed_providers_for(Duration::from_millis(80));
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(
        async move { Agent::new(routed, Store::in_memory()).converse(&mut channel).await },
    );

    owner.says("1");
    let told = owner.hears().await;
    assert!(told.contains("first") && told.contains("second"), "{told}");
    assert_eq!(owner.hears().await, "One.");

    owner.says("2");
    assert_eq!(owner.hears().await, "Two.", "told once, not at every answer");

    tokio::time::sleep(Duration::from_millis(120)).await;
    owner.says("3");
    let back = owner.hears().await;
    assert!(back.contains("first") && back.contains("back"), "{back}");
    assert_eq!(owner.hears().await, "Three.");
}

#[tokio::test]
async fn summaries_are_light_work_and_answers_are_not() {
    let heavy = ScriptedProvider::default();
    heavy.otherwise(|_, _| {
        Ok("A reasonably long answer that takes up some room in the Conversation. ".repeat(4))
    });
    let light = ScriptedProvider::default();
    light.otherwise(|_, _| Ok("SUMMARY".into()));
    let routed = Routed::through("heavy", heavy.clone()).with_light("light", light.clone());
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(async move {
        Agent::new(routed, Store::in_memory())
            .keeping_context_under(6_500)
            .converse(&mut channel)
            .await
    });

    for number in 0..120 {
        owner.says(&format!(
            "Message number {number}, with a little padding to give it some weight."
        ));
        owner.hears().await;
    }

    assert!(light.times_asked() > 0, "no summary was asked of the light model");
    assert!(light.every_call().iter().all(|(instructions, _)| instructions.contains("summar")));
    assert_eq!(heavy.times_asked(), 120);
}

#[tokio::test]
async fn the_owner_can_see_what_was_used_by_provider() {
    let provider = ScriptedProvider::default();
    provider.costing(Usage {
        by: "groq".into(),
        model: "m-1".into(),
        input: 1200,
        cached: 200,
        output: 30,
    });
    provider.then(Ok("One.".into())).then(Ok("Two.".into()));
    let asked = provider.clone();
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(
        async move { Agent::new(provider, Store::in_memory()).converse(&mut channel).await },
    );
    owner.says("1");
    owner.hears().await;
    owner.says("2");
    owner.hears().await;

    owner.says("/usage");
    let shown = owner.hears().await;

    assert!(shown.contains("groq") && shown.contains("m-1"), "{shown}");
    assert!(shown.contains("2 answers"), "{shown}");
    assert!(shown.contains("2,400") && shown.contains("400") && shown.contains("60"), "{shown}");
    assert_eq!(asked.times_asked(), 2, "/usage is not sent to the model");
}
