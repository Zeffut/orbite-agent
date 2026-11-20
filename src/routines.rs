//! Work the Agent does without being asked, on a schedule. A Routine runs in
//! a fresh context, like a Sub-agent, and its report comes back to the Agent,
//! who decides what to tell the Owner.

use std::str::FromStr;

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use croner::Cron;
use serde_json::json;

use crate::ActionSpec;

/// How a moment is written to and by the model.
pub(crate) const MOMENT: &str = "%Y-%m-%d %H:%M";

/// A Routine as kept in the Store.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Routine {
    pub(crate) name: String,
    /// What to do, written to stand on its own.
    pub(crate) instruction: String,
    /// When it recurs, as a five-field cron expression in the Owner's time;
    /// `None` for a Routine that runs once.
    pub(crate) cron: Option<String>,
    /// When it next runs, in seconds since 1970.
    pub(crate) next_run: i64,
    pub(crate) paused: bool,
    pub(crate) last_run: Option<i64>,
    pub(crate) last_result: Option<String>,
}

/// The first moment after `after` that `cron` names.
pub(crate) fn next_after(
    cron: &str,
    after: DateTime<FixedOffset>,
) -> Result<DateTime<FixedOffset>, String> {
    let schedule = Cron::from_str(cron.trim()).map_err(|error| {
        format!(
            "\"{cron}\" is not a valid cron expression ({error}). Give five fields: minute, \
             hour, day of month, month, day of week."
        )
    })?;
    schedule
        .find_next_occurrence(&after, false)
        .map_err(|error| format!("The cron expression \"{cron}\" never comes up ({error})."))
}

/// What `cron` means, in words.
pub(crate) fn in_words(cron: &str) -> String {
    Cron::from_str(cron.trim()).map(|schedule| schedule.describe()).unwrap_or_default()
}

/// The moment written as `text`, in the Owner's time.
pub(crate) fn moment(
    text: &str,
    now: DateTime<FixedOffset>,
) -> Result<DateTime<FixedOffset>, String> {
    let written = NaiveDateTime::parse_from_str(text.trim(), MOMENT)
        .map_err(|_| format!("\"{text}\" is not a moment: write it as YYYY-MM-DD HH:MM."))?;
    let moment = now
        .offset()
        .from_local_datetime(&written)
        .single()
        .ok_or_else(|| format!("\"{text}\" is not a moment that exists."))?;
    if moment <= now {
        return Err(format!("\"{text}\" is in the past: it is now {}.", now.format(MOMENT)));
    }
    Ok(moment)
}

/// `seconds` since 1970 as a moment in the Owner's time.
pub(crate) fn written(seconds: i64, now: DateTime<FixedOffset>) -> String {
    now.offset()
        .timestamp_opt(seconds, 0)
        .single()
        .map(|moment| moment.format(MOMENT).to_string())
        .unwrap_or_default()
}

pub(crate) fn specs() -> Vec<ActionSpec> {
    let text = |description: &str| json!({ "type": "string", "description": description });
    let spec = |name: &str, description: &str, properties: serde_json::Value, required: &[&str]| {
        ActionSpec {
            name: name.into(),
            description: description.into(),
            input: json!({ "type": "object", "properties": properties, "required": required }),
        }
    };
    let named = json!({ "name": text("The routine's name, as listed.") });
    vec![
        spec(
            "schedule_routine",
            "Schedules something for later. Once, with `in_minutes` (a delay, such as \"in ten \
             minutes\") or `at` (a moment): the instruction then comes back to you as a notice \
             and you carry it out. Recurring, with `cron`: each run is done in a fresh context \
             that knows nothing of your conversation and can ask nobody; its report \
             reaches you as a notice. An existing name is replaced.",
            json!({
                "name": text("A few words naming it."),
                "instruction": text("What to do, written to be understood on its own."),
                "cron": text("minute hour day-of-month month day-of-week, in your Owner's time."),
                "at": text("YYYY-MM-DD HH:MM in your Owner's time."),
                "in_minutes": json!({ "type": "number", "description": "Minutes from now." }),
            }),
            &["name", "instruction"],
        ),
        spec(
            "list_routines",
            "Lists the routines, with their schedule, next run and last result.",
            json!({}),
            &[],
        ),
        spec(
            "pause_routine",
            "Stops a routine from running until it is resumed.",
            named.clone(),
            &["name"],
        ),
        spec(
            "resume_routine",
            "Lets a paused routine run again, from its next occurrence.",
            named.clone(),
            &["name"],
        ),
        spec(
            "remove_routine",
            "Deletes a routine. Needs the Owner's approval.",
            named.clone(),
            &["name"],
        ),
        spec(
            "run_routine",
            "Runs a routine now, to try it out, without changing its schedule.",
            named,
            &["name"],
        ),
    ]
}
