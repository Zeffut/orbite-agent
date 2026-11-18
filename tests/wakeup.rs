//! What an Owner observes of the Wake-up: the Agent reviewing, by itself,
//! what is pending.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use orbite::{Agent, Message, Store};
use serde_json::{Value, json};
use support::{Owner, ScriptedProvider, acting, memory_channel, saying};

#[derive(Clone)]
struct Clock(Arc<Mutex<DateTime<FixedOffset>>>);

fn time(written: &str) -> DateTime<FixedOffset> {
    let naive = NaiveDateTime::parse_from_str(written, "%Y-%m-%d %H:%M").unwrap();
    FixedOffset::east_opt(3600).unwrap().from_local_datetime(&naive).unwrap()
}

impl Clock {
    fn at(written: &str) -> Self {
        Self(Arc::new(Mutex::new(time(written))))
    }

    fn set(&self, written: &str) {
        *self.0.lock().unwrap() = time(written);
    }

    fn now(&self) -> DateTime<FixedOffset> {
        *self.0.lock().unwrap()
    }
}

fn is_wakeup(instructions: &str) -> bool {
    instructions.contains("waking up")
}

fn agent_with(provider: &ScriptedProvider, clock: &Clock) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let clock = clock.clone();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .waking_up_every_minutes(240)
            .telling_time_with(move || clock.now())
            .checking_time_every(Duration::from_millis(15))
            .converse(&mut channel)
            .await
    });
    owner
}

/// A model that says `on_wakeup` when woken, carries out the Action the Owner
/// dictates as "name {json}", and relays notices.
fn waking(on_wakeup: &'static str) -> ScriptedProvider {
    let provider = ScriptedProvider::default();
    provider.replying_otherwise(move |instructions, conversation| {
        if is_wakeup(instructions) {
            return Ok(saying(on_wakeup));
        }
        Ok(match conversation.last() {
            Some(Message::Owner(text)) => match text.split_once(' ') {
                Some((name, input)) if input.starts_with('{') => {
                    acting(name, serde_json::from_str::<Value>(input).unwrap())
                }
                _ => saying("Noted."),
            },
            Some(Message::Notice(text)) => saying(&format!("Relayed: {text}")),
            Some(Message::Outcome { text, .. }) => saying(text),
            other => panic!("unexpected: {other:?}"),
        })
    });
    provider
}

fn wakeups(provider: &ScriptedProvider) -> Vec<(String, Vec<Message>)> {
    provider.every_call().into_iter().filter(|(instructions, _)| is_wakeup(instructions)).collect()
}

async fn nothing_for_a_moment(owner: &mut Owner) -> bool {
    tokio::time::sleep(Duration::from_millis(120)).await;
    owner.hears_nothing().await
}

#[tokio::test]
async fn a_wake_up_with_nothing_to_say_sends_nothing_and_leaves_no_trace() {
    let clock = Clock::at("2026-11-19 09:00");
    let provider = waking("[silent]");
    let mut owner = agent_with(&provider, &clock);
    owner.says("I owe Ana a call.");
    owner.hears().await;

    clock.set("2026-11-19 13:30");

    assert!(nothing_for_a_moment(&mut owner).await, "a silent Wake-up wrote to the Owner");
    assert_eq!(wakeups(&provider).len(), 1);
    owner.says("Hi");
    owner.hears().await;
    assert_eq!(
        provider.last_asked(),
        [
            Message::from_owner("I owe Ana a call."),
            Message::from_agent("Noted."),
            Message::from_owner("Hi"),
        ],
        "a Wake-up with nothing to say leaves the Conversation as it was"
    );
}

#[tokio::test]
async fn what_a_wake_up_brings_up_reaches_the_agent_and_the_owner() {
    let clock = Clock::at("2026-11-19 09:00");
    let provider = waking("The Owner still owes Ana a call.");
    let mut owner = agent_with(&provider, &clock);
    owner.says("I owe Ana a call.");
    owner.hears().await;

    clock.set("2026-11-19 13:30");

    let told = owner.hears().await;
    assert!(told.contains("The Owner still owes Ana a call."), "{told}");
}

#[tokio::test]
async fn a_wake_up_is_shown_what_is_recent_and_stays_small_however_long_the_conversation() {
    let clock = Clock::at("2026-11-19 09:00");
    let provider = waking("[silent]");
    let mut owner = agent_with(&provider, &clock);
    for number in 0..40 {
        owner.says(&format!(
            "Fact number {number}: {}",
            "some words to give it weight. ".repeat(12)
        ));
        owner.hears().await;
    }
    owner.says("And I owe Ana a call.");
    owner.hears().await;

    clock.set("2026-11-19 13:30");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let woken = wakeups(&provider);
    let (instructions, given) = woken.last().expect("no Wake-up happened");
    let [Message::Owner(material)] = &given[..] else { panic!("{given:?}") };
    assert!(material.contains("I owe Ana a call."), "{material}");
    let tokens = (instructions.len() + material.len()) / 4;
    assert!(tokens < 2_000, "a Wake-up sent about {tokens} tokens");
    let offered: Vec<_> = provider
        .every_offer()
        .into_iter()
        .filter(|(instructions, _)| is_wakeup(instructions))
        .collect();
    assert!(offered.iter().all(|(_, actions)| actions.is_empty()), "a Wake-up only thinks");
}

#[tokio::test]
async fn wake_ups_come_at_their_pace_counted_from_the_owners_last_word() {
    let clock = Clock::at("2026-11-19 09:00");
    let provider = waking("[silent]");
    let mut owner = agent_with(&provider, &clock);
    owner.says("Hi");
    owner.hears().await;

    clock.set("2026-11-19 12:59");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(wakeups(&provider).len(), 0, "it woke up before its pace of four hours");

    clock.set("2026-11-19 13:00");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(wakeups(&provider).len(), 1);

    clock.set("2026-11-19 14:00");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(wakeups(&provider).len(), 1, "it woke up again too soon");

    // The Owner writes: the next Wake-up is counted from there.
    clock.set("2026-11-19 16:30");
    owner.says("Hi again");
    owner.hears().await;
    clock.set("2026-11-19 17:30");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(wakeups(&provider).len(), 1);
    clock.set("2026-11-19 20:30");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(wakeups(&provider).len(), 2);
}

#[tokio::test]
async fn the_owner_sets_the_pace_of_wake_ups_or_turns_them_off() {
    let clock = Clock::at("2026-11-19 09:00");
    let provider = waking("[silent]");
    let mut owner = agent_with(&provider, &clock);

    owner.says(&format!("set_wake_ups {}", json!({ "every_minutes": 30 })));
    assert!(owner.hears().await.contains("30 minutes"));
    clock.set("2026-11-19 09:30");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(wakeups(&provider).len(), 1);

    owner.says(&format!("set_wake_ups {}", json!({ "every_minutes": 0 })));
    assert!(owner.hears().await.contains("off"));
    clock.set("2026-11-20 09:30");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(wakeups(&provider).len(), 1);
}

#[tokio::test]
async fn the_agent_does_not_wake_up_during_quiet_hours() {
    let clock = Clock::at("2026-11-19 21:00");
    let provider = waking("Something.");
    let mut owner = agent_with(&provider, &clock);
    owner.says(&format!("set_quiet_hours {}", json!({ "from": "22:00", "until": "07:00" })));
    owner.hears().await;

    clock.set("2026-11-20 03:00");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(wakeups(&provider).len(), 0);

    clock.set("2026-11-20 07:05");
    assert!(owner.hears().await.contains("Something."));
}

#[tokio::test]
async fn a_wake_up_that_fails_troubles_nobody() {
    let clock = Clock::at("2026-11-19 09:00");
    let provider = ScriptedProvider::default();
    provider.replying_otherwise(|instructions, _| {
        if is_wakeup(instructions) {
            return Err(orbite::ProviderError::Failed("model crashed".into()));
        }
        Ok(saying("Noted."))
    });
    let mut owner = agent_with(&provider, &clock);
    owner.says("Hi");
    owner.hears().await;

    clock.set("2026-11-19 13:30");

    assert!(nothing_for_a_moment(&mut owner).await);
    owner.says("Still there?");
    assert_eq!(owner.hears().await, "Noted.");
}
