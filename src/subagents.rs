//! Temporary workers the Agent starts for long or parallel pieces of work.
//! Each has its own short conversation, works in the background, reports
//! back once and disappears.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::actions::{Host, Review};
use crate::agent::{clip, explain, patiently};
use crate::{ActionSpec, Message, Provider, Request};

/// How many Sub-agents may work at the same time.
const AT_ONCE: usize = 4;
/// A Sub-agent still working after this long is stopped.
const TIME_LIMIT: Duration = Duration::from_secs(15 * 60);
/// How much of an Outcome a Sub-agent is given.
const OUTCOME_KEPT: usize = 20_000;

/// Actions that are the Agent's alone: a Sub-agent neither starts others nor
/// changes what the Agent remembers or knows how to do.
const NOT_FOR_SUBAGENTS: &[&str] = &[
    "delegate",
    "list_subagents",
    "stop_subagent",
    "search_conversation",
    "remember",
    "forget",
    "save_skill",
    "remove_skill",
];

const INSTRUCTIONS: &str = "\
You are a sub-agent of a personal AI agent, started for one task. You cannot \
talk to anyone: your only output is the report you end with, which the agent \
that started you will read. Work on the task with the actions you are given. \
You can look at things but not change them: an action that needs the Owner's \
approval will not be carried out, so say in your report what would need doing. \
When you are done, reply with the report alone: what you found or did, what \
you could not do and why. Be precise and brief.";

/// What a Sub-agent, or the run of a Routine, ended with.
pub(crate) struct Report {
    /// The Routine this was a run of, if any.
    pub(crate) routine: Option<String>,
    /// How it ended and what it reported, without saying who it is.
    pub(crate) ended: String,
    /// Who it is, such as `Sub-agent "counter"`.
    pub(crate) who: String,
}

pub(crate) struct Subagents {
    running: HashMap<String, Running>,
    reports: mpsc::UnboundedSender<Report>,
    /// How many turns a Sub-agent gets before it must report.
    pub(crate) steps: usize,
}

/// Why a Sub-agent could not start.
enum Busy {
    SameName,
    Full,
}

struct Running {
    task: String,
    work: JoinHandle<()>,
    since: Instant,
}

impl Subagents {
    /// The Sub-agents, and where their reports arrive.
    pub(crate) fn new() -> (Self, mpsc::UnboundedReceiver<Report>) {
        let (reports, arriving) = mpsc::unbounded_channel();
        (Self { running: HashMap::new(), reports, steps: 25 }, arriving)
    }

    pub(crate) fn specs() -> Vec<ActionSpec> {
        let spec = |name: &str, description: &str, properties: serde_json::Value| {
            let required: Vec<&String> = properties.as_object().unwrap().keys().collect();
            ActionSpec {
                name: name.into(),
                description: description.into(),
                input: serde_json::json!({ "type": "object", "properties": properties, "required": required }),
            }
        };
        let text =
            |description: &str| serde_json::json!({ "type": "string", "description": description });
        vec![
            spec(
                "delegate",
                "Starts a sub-agent on a long or parallel task and returns at once; its report \
                 reaches you later as a notice. It knows nothing of your conversation and only \
                 looks at things.",
                serde_json::json!({
                    "name": text("A short name for it."),
                    "task": text("Everything it needs: what to do and what to report."),
                }),
            ),
            spec("list_subagents", "Lists the sub-agents now working.", serde_json::json!({})),
            spec(
                "stop_subagent",
                "Stops a sub-agent. Nothing of its work is reported.",
                serde_json::json!({ "name": text("The sub-agent's name.") }),
            ),
        ]
    }

    /// Starts a Sub-agent on `task`; the text is the Outcome for the model.
    pub(crate) fn start<P: Provider + 'static>(
        &mut self,
        name: &str,
        task: &str,
        provider: Arc<P>,
        host: Arc<Host>,
    ) -> String {
        let name = name.trim().to_owned();
        if name.is_empty() || task.trim().is_empty() {
            return "A sub-agent needs a name and a task.".into();
        }
        let who = format!("Sub-agent \"{name}\"");
        match self.launch(name.clone(), who, None, task, provider, host) {
            Ok(()) => format!(
                "Sub-agent \"{name}\" started. Its report will reach you as a notice when it is \
                 done; carry on meanwhile."
            ),
            Err(Busy::SameName) => {
                format!("A sub-agent called \"{name}\" is already working. Choose another name.")
            }
            Err(Busy::Full) => {
                format!("{AT_ONCE} sub-agents are already working. Wait for one to finish.")
            }
        }
    }

    /// Starts a run of the Routine `routine`, announced as `who`. False when
    /// it could not start now: a run of it is still going, or too much is.
    pub(crate) fn run_routine<P: Provider + 'static>(
        &mut self,
        routine: &str,
        who: String,
        task: &str,
        provider: Arc<P>,
        host: Arc<Host>,
    ) -> bool {
        let key = format!("routine: {routine}");
        self.launch(key, who, Some(routine.to_owned()), task, provider, host).is_ok()
    }

    fn launch<P: Provider + 'static>(
        &mut self,
        key: String,
        who: String,
        routine: Option<String>,
        task: &str,
        provider: Arc<P>,
        host: Arc<Host>,
    ) -> Result<(), Busy> {
        self.forget_finished();
        if self.running.contains_key(&key) {
            return Err(Busy::SameName);
        }
        if self.running.len() >= AT_ONCE {
            return Err(Busy::Full);
        }
        let (reports, steps, given) = (self.reports.clone(), self.steps, task.to_owned());
        let work = tokio::spawn(async move {
            let working = work(&*provider, &host, &given, steps);
            let ended = match tokio::time::timeout(TIME_LIMIT, working).await {
                Ok(ended) => ended,
                Err(_) => "ran out of time and was stopped. It reported nothing.".into(),
            };
            let _ = reports.send(Report { routine, ended, who });
        });
        self.running.insert(key, Running { task: task.to_owned(), work, since: Instant::now() });
        Ok(())
    }

    pub(crate) fn list(&mut self) -> String {
        self.forget_finished();
        if self.running.is_empty() {
            return "No sub-agent is working.".into();
        }
        let mut lines: Vec<String> = self
            .running
            .iter()
            .map(|(name, running)| {
                let minutes = running.since.elapsed().as_secs() / 60;
                format!("- {name} (for {minutes} min): {}", clip(&running.task, 300))
            })
            .collect();
        lines.sort();
        lines.join("\n")
    }

    pub(crate) fn stop(&mut self, name: &str) -> String {
        self.forget_finished();
        match self.running.remove(name.trim()) {
            Some(running) => {
                running.work.abort();
                format!("Stopped the sub-agent \"{}\".", name.trim())
            }
            None => format!("No sub-agent called \"{}\" is working.", name.trim()),
        }
    }

    fn forget_finished(&mut self) {
        self.running.retain(|_, running| !running.work.is_finished());
    }
}

impl Drop for Subagents {
    fn drop(&mut self) {
        for running in self.running.values() {
            running.work.abort();
        }
    }
}

/// Works on `task` to its end; the text says how it ended and carries the report.
async fn work(provider: &impl Provider, host: &Host, task: &str, steps: usize) -> String {
    let actions: Vec<ActionSpec> = host
        .specs()
        .into_iter()
        .filter(|action| !NOT_FOR_SUBAGENTS.contains(&action.name.as_str()))
        .collect();
    let mut conversation = vec![Message::from_owner(task)];
    for _ in 0..steps {
        let request = Request {
            instructions: INSTRUCTIONS,
            conversation: &conversation,
            actions: &actions,
            images: &[],
        };
        let reply = match patiently(provider, request, host.busy_wait).await {
            Ok(reply) => reply,
            Err(error) => return format!("failed: {}", explain(&error)),
        };
        conversation
            .push(Message::Agent { text: reply.text.clone(), actions: reply.actions.clone() });
        if reply.actions.is_empty() {
            return format!("finished. Its report:\n{}", reply.text);
        }
        for action in &reply.actions {
            let text = if NOT_FOR_SUBAGENTS.contains(&action.name.as_str()) {
                format!("There is no Action called {}.", action.name)
            } else {
                match host.review(action) {
                    Review::Free => host.carry_out(action).await,
                    Review::Refuse(problem) => problem,
                    Review::Ask(_) => "This needs the Owner's approval, which a sub-agent cannot \
                                       ask for. It was not carried out: say in your report what \
                                       would need doing."
                        .into(),
                }
            };
            let text = clip(&text, OUTCOME_KEPT).to_owned();
            conversation.push(Message::Outcome { action: action.id.clone(), text });
        }
    }
    conversation.push(Message::from_owner(
        "You are out of steps. Do nothing more: report now what you found and what is left to do.",
    ));
    let request = Request {
        instructions: INSTRUCTIONS,
        conversation: &conversation,
        actions: &[],
        images: &[],
    };
    match patiently(provider, request, host.busy_wait).await {
        Ok(reply) => format!("ran out of steps. Its report so far:\n{}", reply.text),
        Err(error) => format!("ran out of steps, then failed: {}", explain(&error)),
    }
}
