//! What an Owner observes when the Agent writes first.

mod support;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use orbite::{Agent, Message, Reply, Store};
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

fn agent_with(provider: &ScriptedProvider, store: Store, clock: &Clock, workspace: &Path) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let clock = clock.clone();
    let workspace = workspace.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, store)
            .working_in(workspace)
            .telling_time_with(move || clock.now())
            .checking_time_every(Duration::from_millis(15))
            .converse(&mut channel)
            .await
    });
    owner
}

/// An Agent whose model carries out the Action the Owner dictates as
/// "name {json}", repeats Outcomes, and answers a notice with `on_notice`.
fn obedient(on_notice: impl Fn(&str) -> Reply + Send + 'static) -> ScriptedProvider {
    let provider = ScriptedProvider::default();
    provider.replying_otherwise(move |_, conversation| {
        Ok(match conversation.last() {
            Some(Message::Owner(text)) => match text.split_once(' ') {
                Some((name, input)) if input.starts_with('{') => {
                    acting(name, serde_json::from_str::<Value>(input).unwrap())
                }
                _ => saying("Hello."),
            },
            Some(Message::Notice(text)) => on_notice(text),
            Some(Message::Outcome { text, .. }) => saying(&format!("Outcome: {text}")),
            other => panic!("unexpected: {other:?}"),
        })
    });
    provider
}

async fn dictate(owner: &mut Owner, action: &str, input: Value) -> String {
    owner.says(&format!("{action} {input}"));
    owner.hears().await
}

async fn remind_at(owner: &mut Owner, at: &str) {
    dictate(
        owner,
        "schedule_routine",
        json!({ "name": "ping", "instruction": "Say ping.", "at": at }),
    )
    .await;
}

async fn nothing_for_a_moment(owner: &mut Owner) -> bool {
    tokio::time::sleep(Duration::from_millis(120)).await;
    owner.hears_nothing().await
}

#[tokio::test]
async fn the_agent_can_take_note_of_something_without_writing_to_the_owner() {
    let clock = Clock::at("2026-11-19 10:00");
    let workspace = tempfile::tempdir().unwrap();
    let provider = obedient(|_| saying("[silent]"));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock, workspace.path());
    remind_at(&mut owner, "2026-11-19 10:30").await;

    clock.set("2026-11-19 10:30");

    assert!(nothing_for_a_moment(&mut owner).await, "a silent answer was sent");
    owner.says("Hi");
    assert_eq!(owner.hears().await, "Hello.");
}

#[tokio::test]
async fn what_the_agent_writes_first_waits_for_the_end_of_quiet_hours() {
    let clock = Clock::at("2026-11-19 21:00");
    let workspace = tempfile::tempdir().unwrap();
    let provider = obedient(|_| saying("Ping."));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock, workspace.path());
    let set =
        dictate(&mut owner, "set_quiet_hours", json!({ "from": "22:00", "until": "07:00" })).await;
    assert!(set.contains("22:00") && set.contains("07:00"), "{set}");
    remind_at(&mut owner, "2026-11-19 23:30").await;

    clock.set("2026-11-19 23:30");
    assert!(nothing_for_a_moment(&mut owner).await, "the Owner was written to during quiet hours");
    clock.set("2026-11-20 06:59");
    assert!(nothing_for_a_moment(&mut owner).await, "quiet hours are not over");

    clock.set("2026-11-20 07:00");
    assert_eq!(owner.hears().await, "Ping.");
    assert!(nothing_for_a_moment(&mut owner).await, "it was delivered twice");
}

#[tokio::test]
async fn several_held_messages_arrive_together() {
    let clock = Clock::at("2026-11-19 21:00");
    let workspace = tempfile::tempdir().unwrap();
    let count = std::sync::atomic::AtomicUsize::new(0);
    let provider = obedient(move |_| {
        saying(&format!("Note {}.", count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1))
    });
    let mut owner = agent_with(&provider, Store::in_memory(), &clock, workspace.path());
    dictate(&mut owner, "set_quiet_hours", json!({ "from": "22:00", "until": "07:00" })).await;
    remind_at(&mut owner, "2026-11-19 23:00").await;
    dictate(
        &mut owner,
        "schedule_routine",
        json!({ "name": "pong", "instruction": "x", "at": "2026-11-20 02:00" }),
    )
    .await;
    clock.set("2026-11-19 23:00");
    tokio::time::sleep(Duration::from_millis(150)).await;
    clock.set("2026-11-20 02:00");
    tokio::time::sleep(Duration::from_millis(150)).await;

    clock.set("2026-11-20 07:30");

    assert_eq!(owner.hears().await, "Note 1.\n\nNote 2.");
}

#[tokio::test]
async fn what_is_urgent_is_delivered_even_during_quiet_hours() {
    let clock = Clock::at("2026-11-19 21:00");
    let workspace = tempfile::tempdir().unwrap();
    let provider = obedient(|_| saying("[urgent] The house alarm is ringing."));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock, workspace.path());
    dictate(&mut owner, "set_quiet_hours", json!({ "from": "22:00", "until": "07:00" })).await;
    remind_at(&mut owner, "2026-11-19 23:30").await;

    clock.set("2026-11-19 23:30");

    assert_eq!(owner.hears().await, "The house alarm is ringing.");
}

#[tokio::test]
async fn the_owner_is_always_answered_and_gets_what_was_held_when_they_write() {
    let clock = Clock::at("2026-11-19 21:00");
    let workspace = tempfile::tempdir().unwrap();
    let provider = obedient(|_| saying("Ping."));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock, workspace.path());
    dictate(&mut owner, "set_quiet_hours", json!({ "from": "22:00", "until": "07:00" })).await;
    remind_at(&mut owner, "2026-11-19 23:30").await;
    clock.set("2026-11-19 23:30");
    assert!(nothing_for_a_moment(&mut owner).await);

    // Awake after all.
    owner.says("Hi");

    assert_eq!(owner.hears().await, "Ping.");
    assert_eq!(owner.hears().await, "Hello.");
}

#[tokio::test]
async fn nothing_needing_an_approval_is_asked_during_quiet_hours_on_the_agents_initiative() {
    let clock = Clock::at("2026-11-19 21:00");
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.replying_otherwise(|_, conversation| {
        Ok(match conversation.last() {
            Some(Message::Owner(text)) => match text.split_once(' ') {
                Some((name, input)) if input.starts_with('{') => {
                    acting(name, serde_json::from_str::<Value>(input).unwrap())
                }
                _ => saying("Hello."),
            },
            Some(Message::Notice(_)) => {
                acting("write_file", json!({ "path": "made.txt", "content": "x" }))
            }
            Some(Message::Outcome { text, .. }) if text.contains("quiet") => {
                saying("[urgent] Deferred.")
            }
            Some(Message::Outcome { text, .. }) => saying(text),
            other => panic!("unexpected: {other:?}"),
        })
    });
    let mut owner = agent_with(&provider, Store::in_memory(), &clock, workspace.path());
    dictate(&mut owner, "set_quiet_hours", json!({ "from": "22:00", "until": "07:00" })).await;
    remind_at(&mut owner, "2026-11-19 23:30").await;

    clock.set("2026-11-19 23:30");

    assert_eq!(owner.hears().await, "Deferred.");
    assert!(owner.was_asked_nothing(), "an Approval was asked for during quiet hours");
    assert!(!workspace.path().join("made.txt").exists());
}

#[tokio::test]
async fn quiet_hours_can_be_turned_off_and_wrong_times_are_explained() {
    let clock = Clock::at("2026-11-19 21:00");
    let workspace = tempfile::tempdir().unwrap();
    let provider = obedient(|_| saying("Ping."));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock, workspace.path());

    let wrong =
        dictate(&mut owner, "set_quiet_hours", json!({ "from": "ten", "until": "07:00" })).await;
    assert!(wrong.contains("HH:MM"), "{wrong}");

    dictate(&mut owner, "set_quiet_hours", json!({ "from": "22:00", "until": "07:00" })).await;
    let off = dictate(&mut owner, "set_quiet_hours", json!({ "from": "", "until": "" })).await;
    assert!(off.contains("off"), "{off}");
    remind_at(&mut owner, "2026-11-19 23:30").await;
    clock.set("2026-11-19 23:30");

    assert_eq!(owner.hears().await, "Ping.");
}

#[tokio::test]
async fn what_was_held_survives_a_restart() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("orbite.db");
    let clock = Clock::at("2026-11-19 21:00");
    let provider = obedient(|_| saying("Ping."));
    {
        let mut owner = agent_with(&provider, Store::open(&database).unwrap(), &clock, home.path());
        dictate(&mut owner, "set_quiet_hours", json!({ "from": "22:00", "until": "07:00" })).await;
        remind_at(&mut owner, "2026-11-19 23:30").await;
        clock.set("2026-11-19 23:30");
        assert!(nothing_for_a_moment(&mut owner).await);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    clock.set("2026-11-20 08:00");
    let mut owner = agent_with(&provider, Store::open(&database).unwrap(), &clock, home.path());

    assert_eq!(owner.hears().await, "Ping.");
}

#[tokio::test]
async fn the_model_is_told_the_quiet_hours_and_whether_it_is_quiet_now() {
    let clock = Clock::at("2026-11-19 23:00");
    let workspace = tempfile::tempdir().unwrap();
    let provider = obedient(|_| saying("Ping."));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock, workspace.path());
    dictate(&mut owner, "set_quiet_hours", json!({ "from": "22:00", "until": "07:00" })).await;

    owner.says("Hi");
    owner.hears().await;

    let instructions = provider.last_instructions();
    assert!(instructions.contains("22:00") && instructions.contains("07:00"), "{instructions}");
    assert!(instructions.contains("quiet hours now"), "{instructions}");
}
