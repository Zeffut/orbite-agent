//! What an Owner observes of Routines: work the Agent does without being asked.

mod support;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use orbite::{Agent, Message, ProviderError, Reply, Store};
use serde_json::{Value, json};
use support::{Owner, ScriptedProvider, acting, memory_channel, saying};

/// A clock the test sets by hand, one hour east of UTC.
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

fn is_worker(instructions: &str) -> bool {
    instructions.contains("sub-agent of")
}

fn agent_with(provider: &ScriptedProvider, store: Store, clock: &Clock) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let clock = clock.clone();
    tokio::spawn(async move {
        Agent::new(provider, store)
            .telling_time_with(move || clock.now())
            .checking_time_every(Duration::from_millis(15))
            .converse(&mut channel)
            .await
    });
    owner
}

/// An Agent whose model carries out whatever Action the Owner dictates as
/// "name {json}", repeats Outcomes, and relays notices; `worker` plays the
/// fresh context a Routine runs in.
fn obedient(
    worker: impl Fn(&[Message]) -> Result<Reply, ProviderError> + Send + 'static,
) -> ScriptedProvider {
    let provider = ScriptedProvider::default();
    provider.replying_otherwise(move |instructions, conversation| {
        if is_worker(instructions) {
            return worker(conversation);
        }
        Ok(match conversation.last() {
            Some(Message::Owner(text)) => match text.split_once(' ') {
                Some((name, input)) if input.starts_with('{') => {
                    acting(name, serde_json::from_str::<Value>(input).unwrap())
                }
                _ => saying("Hello."),
            },
            Some(Message::Outcome { text, .. }) => saying(text),
            Some(Message::Notice(text)) => saying(&format!("Relayed: {text}")),
            other => panic!("unexpected: {other:?}"),
        })
    });
    provider
}

async fn dictate(owner: &mut Owner, action: &str, input: Value) -> String {
    owner.says(&format!("{action} {input}"));
    owner.hears().await
}

/// Schedules the Routine "briefing" every day at 09:00, with the Owner's Approval.
async fn schedule_briefing(owner: &mut Owner) {
    owner.says(&format!(
        "schedule_routine {}",
        json!({ "name": "briefing", "instruction": "Check the weather.", "cron": "0 9 * * *" })
    ));
    owner.is_asked().await;
    owner.allows();
    owner.hears().await;
}

async fn nothing_for_a_moment(owner: &mut Owner) -> bool {
    tokio::time::sleep(Duration::from_millis(120)).await;
    owner.hears_nothing().await
}

#[tokio::test]
async fn a_routine_runs_at_its_time_and_the_agent_tells_the_owner() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny, 12 degrees.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);
    schedule_briefing(&mut owner).await;
    assert!(nothing_for_a_moment(&mut owner).await, "it ran before its time");

    clock.set("2026-11-19 09:00");

    let told = owner.hears().await;
    assert!(told.contains("briefing") && told.contains("Sunny, 12 degrees."), "{told}");
}

#[tokio::test]
async fn a_routine_runs_in_a_fresh_context_with_only_its_instruction() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);
    schedule_briefing(&mut owner).await;

    clock.set("2026-11-19 09:00");
    owner.hears().await;

    let (_, given) = provider
        .every_call()
        .into_iter()
        .find(|(instructions, _)| is_worker(instructions))
        .unwrap();
    let [Message::Owner(task)] = &given[..] else { panic!("{given:?}") };
    assert!(task.contains("Check the weather."), "{task}");
}

#[tokio::test]
async fn a_routine_runs_once_per_occurrence_and_again_at_the_next() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);
    schedule_briefing(&mut owner).await;

    clock.set("2026-11-19 09:00");
    owner.hears().await;
    clock.set("2026-11-19 09:01");
    assert!(nothing_for_a_moment(&mut owner).await, "it ran twice for one occurrence");

    clock.set("2026-11-20 09:00");
    owner.hears().await;
}

#[tokio::test]
async fn a_recurring_routine_waits_for_the_owners_approval() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);

    owner.says(&format!(
        "schedule_routine {}",
        json!({ "name": "briefing", "instruction": "Check the weather.", "cron": "0 9 * * *" })
    ));
    let approval = owner.is_asked().await;
    assert!(approval.detail.contains("Check the weather."), "{approval:?}");
    assert!(approval.detail.contains("2026-11-19 09:00"), "its next run is shown: {approval:?}");
    owner.denies();
    owner.hears().await;

    clock.set("2026-11-19 09:00");
    assert!(nothing_for_a_moment(&mut owner).await, "a denied Routine ran");
}

#[tokio::test]
async fn a_reminder_set_for_one_moment_needs_no_approval_runs_once_and_is_gone() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Time to call Ana.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);

    let set = dictate(
        &mut owner,
        "schedule_routine",
        json!({ "name": "call ana", "instruction": "Remind the Owner to call Ana.", "at": "2026-11-19 17:30" }),
    )
    .await;
    assert!(owner.was_asked_nothing());
    assert!(set.contains("2026-11-19 17:30"), "{set}");

    clock.set("2026-11-19 17:30");
    let told = owner.hears().await;
    assert!(told.contains("Remind the Owner to call Ana."), "{told}");
    assert!(
        !provider.every_call().iter().any(|(instructions, _)| is_worker(instructions)),
        "a single reminder goes straight to the Agent, with no run in a fresh context"
    );

    let listed = dictate(&mut owner, "list_routines", json!({})).await;
    assert!(listed.contains("No routine"), "{listed}");
}

#[tokio::test]
async fn routines_are_listed_with_their_schedule_next_run_and_last_result() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);
    schedule_briefing(&mut owner).await;

    let before = dictate(&mut owner, "list_routines", json!({})).await;
    assert!(before.contains("briefing"), "{before}");
    assert!(before.contains("0 9 * * *"), "{before}");
    assert!(before.contains("Check the weather."), "{before}");
    assert!(before.contains("next: 2026-11-19 09:00"), "{before}");

    clock.set("2026-11-19 09:00");
    owner.hears().await;

    let after = dictate(&mut owner, "list_routines", json!({})).await;
    assert!(after.contains("next: 2026-11-20 09:00"), "{after}");
    assert!(after.contains("last: 2026-11-19 09:00") && after.contains("Sunny."), "{after}");
}

#[tokio::test]
async fn a_paused_routine_does_not_run_until_it_is_resumed() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);
    schedule_briefing(&mut owner).await;

    dictate(&mut owner, "pause_routine", json!({ "name": "briefing" })).await;
    clock.set("2026-11-19 09:00");
    assert!(nothing_for_a_moment(&mut owner).await, "a paused Routine ran");

    dictate(&mut owner, "resume_routine", json!({ "name": "briefing" })).await;
    assert!(nothing_for_a_moment(&mut owner).await, "what was missed while paused is not made up");
    clock.set("2026-11-20 09:00");
    owner.hears().await;
}

#[tokio::test]
async fn removing_a_routine_waits_for_the_owner() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);
    schedule_briefing(&mut owner).await;

    owner.says(&format!("remove_routine {}", json!({ "name": "briefing" })));
    let approval = owner.is_asked().await;
    assert!(approval.irreversible, "{approval:?}");
    owner.allows();
    owner.hears().await;

    clock.set("2026-11-19 09:00");
    assert!(nothing_for_a_moment(&mut owner).await);
}

#[tokio::test]
async fn scheduling_under_an_existing_name_replaces_the_routine() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);
    schedule_briefing(&mut owner).await;

    owner.says(&format!(
        "schedule_routine {}",
        json!({ "name": "briefing", "instruction": "Check the trains.", "cron": "30 7 * * *" })
    ));
    let approval = owner.is_asked().await;
    assert!(approval.summary.contains("Replace"), "{approval:?}");
    owner.allows();
    owner.hears().await;

    let listed = dictate(&mut owner, "list_routines", json!({})).await;
    assert!(listed.contains("Check the trains.") && !listed.contains("weather"), "{listed}");
    assert!(listed.contains("next: 2026-11-20 07:30"), "{listed}");
}

#[tokio::test]
async fn a_schedule_that_makes_no_sense_is_explained_to_the_model() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);
    for (input, expected) in [
        (json!({ "name": "x", "instruction": "i", "cron": "every morning" }), "cron"),
        (json!({ "name": "x", "instruction": "i", "at": "2026-11-18 09:00" }), "past"),
        (json!({ "name": "x", "instruction": "i", "at": "tomorrow" }), "YYYY-MM-DD HH:MM"),
        (json!({ "name": "x", "instruction": "i" }), "cron"),
        (
            json!({ "name": "x", "instruction": "i", "cron": "0 9 * * *", "at": "2026-11-20 09:00" }),
            "not several",
        ),
    ] {
        let explained = dictate(&mut owner, "schedule_routine", input).await;
        assert!(explained.contains(expected), "{explained}");
        assert!(owner.was_asked_nothing());
    }
}

#[tokio::test]
async fn a_routine_that_fails_tells_the_owner_why() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Err(ProviderError::Failed("model crashed".into())));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);
    schedule_briefing(&mut owner).await;

    clock.set("2026-11-19 09:00");

    let told = owner.hears().await;
    assert!(
        told.contains("briefing") && told.contains("failed") && told.contains("model crashed"),
        "{told}"
    );
    let listed = dictate(&mut owner, "list_routines", json!({})).await;
    assert!(listed.contains("failed"), "the failure is kept as the last result: {listed}");
}

#[tokio::test]
async fn routines_survive_a_restart() {
    let home = tempfile::tempdir().unwrap();
    let database: &Path = &home.path().join("orbite.db");
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    {
        let mut owner = agent_with(&provider, Store::open(database).unwrap(), &clock);
        schedule_briefing(&mut owner).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut owner = agent_with(&provider, Store::open(database).unwrap(), &clock);
    clock.set("2026-11-19 09:00");

    assert!(owner.hears().await.contains("Sunny."));
}

#[tokio::test]
async fn a_routine_missed_while_orbite_was_stopped_runs_once_at_the_next_start_and_says_it_is_late()
{
    let home = tempfile::tempdir().unwrap();
    let database: &Path = &home.path().join("orbite.db");
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    {
        let mut owner = agent_with(&provider, Store::open(database).unwrap(), &clock);
        schedule_briefing(&mut owner).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Three occurrences pass while Orbite is stopped.
    clock.set("2026-11-22 10:30");
    let mut owner = agent_with(&provider, Store::open(database).unwrap(), &clock);

    let told = owner.hears().await;
    assert!(told.contains("late"), "{told}");
    assert!(nothing_for_a_moment(&mut owner).await, "missed occurrences are not all made up");
    let listed = dictate(&mut owner, "list_routines", json!({})).await;
    assert!(listed.contains("next: 2026-11-23 09:00"), "{listed}");
}

#[tokio::test]
async fn a_routine_can_be_tried_out_now_and_the_agent_says_it_only_looks() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Sunny.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);
    schedule_briefing(&mut owner).await;

    let started = dictate(&mut owner, "run_routine", json!({ "name": "briefing" })).await;
    assert!(started.contains("look"), "what a run can and cannot do is stated: {started}");

    assert!(owner.hears().await.contains("Sunny."));
    let listed = dictate(&mut owner, "list_routines", json!({})).await;
    assert!(
        listed.contains("next: 2026-11-19 09:00"),
        "a try-out does not move the schedule: {listed}"
    );
}

#[tokio::test]
async fn the_model_is_told_the_date_and_time() {
    let clock = Clock::at("2026-11-19 08:05");
    let provider = obedient(|_| Ok(saying("x")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);

    owner.says("Hi");
    owner.hears().await;

    let instructions = provider.last_instructions();
    assert!(
        instructions.contains("2026-11-19 08:05") && instructions.contains("Thursday"),
        "{instructions}"
    );
}

#[tokio::test]
async fn a_reminder_asked_for_in_a_few_minutes_comes_that_long_after_to_the_second() {
    let clock = Clock::at("2026-11-19 08:00");
    // Fifty seconds into the minute, as a request usually is.
    let later = |seconds: i64| time("2026-11-19 08:00") + chrono::Duration::seconds(seconds);
    *clock.0.lock().unwrap() = later(50);
    let provider = obedient(|_| Ok(saying("Time to water the plants.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);

    let set = dictate(
        &mut owner,
        "schedule_routine",
        json!({ "name": "plants", "instruction": "Remind the Owner to water the plants.", "in_minutes": 3 }),
    )
    .await;
    assert!(set.contains("08:03"), "{set}");

    // Three minutes are not over when the clock shows 08:03.
    *clock.0.lock().unwrap() = later(3 * 60 + 20);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let listed = dictate(&mut owner, "list_routines", json!({})).await;
    assert!(listed.contains("plants"), "it came early: {listed}");

    *clock.0.lock().unwrap() = later(3 * 60 + 50);
    let told = owner.hears().await;
    assert!(told.contains("water the plants"), "{told}");
}

#[tokio::test]
async fn a_moment_and_a_delay_are_not_both_given() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Done.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);

    let refused = dictate(
        &mut owner,
        "schedule_routine",
        json!({ "name": "x", "instruction": "i", "in_minutes": 3, "at": "2026-11-19 09:00" }),
    )
    .await;
    assert!(refused.contains("one of"), "{refused}");
    let refused = dictate(
        &mut owner,
        "schedule_routine",
        json!({ "name": "x", "instruction": "i", "in_minutes": 0 }),
    )
    .await;
    assert!(refused.contains("in_minutes"), "{refused}");
}

#[tokio::test]
async fn a_model_that_fills_in_every_field_still_schedules_what_it_means() {
    let clock = Clock::at("2026-11-19 08:00");
    let provider = obedient(|_| Ok(saying("Done.")));
    let mut owner = agent_with(&provider, Store::in_memory(), &clock);

    // Some models write every field of an Action, empty or zero when unused.
    owner.says(&format!(
        "schedule_routine {}",
        json!({ "name": "tokyo", "instruction": "Say the time in Tokyo.", "cron": "0 9 * * *", "at": "", "in_minutes": 0 })
    ));
    owner.is_asked().await;
    owner.allows();
    owner.hears().await;
    let listed = dictate(&mut owner, "list_routines", json!({})).await;
    assert!(listed.contains("tokyo") && listed.contains("0 9 * * *"), "{listed}");

    let set = dictate(
        &mut owner,
        "schedule_routine",
        json!({ "name": "tea", "instruction": "Remind the Owner of the tea.", "cron": "", "at": "", "in_minutes": 5 }),
    )
    .await;
    assert!(set.contains("08:05"), "{set}");
}
