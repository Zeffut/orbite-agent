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
/// How many of the latest entries of its work a Sub-agent is shown the
/// pictures of.
const SHOWING_PICTURES: usize = 6;
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
that started you will read. Work on the task with the actions you are given, \
to its end: you have many steps, use them. When one way does not work, try \
another before giving up on it: a page that cannot be read is opened on the \
Computer and read from its screen; a search that finds nothing is asked \
again in other words; a file you may not write here is written elsewhere in \
your workspace. Report as not done only what you really tried several ways. \
You cannot ask the Owner for anything: an action that would need their \
approval is not carried out, so say in your report what would need doing. \
When you are done, reply with the report alone: what you found or did, with \
the facts and figures themselves, then what you could not do and exactly why \
(what you tried, and what answered). Be precise and brief.";

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
    /// What the Owner lets be done without asking, as it stood when the
    /// Agent last looked: those it starts are held to the same.
    /// Shared with those at work: what the Owner tightens holds for them
    /// at once.
    allowed: Arc<std::sync::RwLock<Allowance>>,
}

/// What the Owner lets be done without asking them.
#[derive(Clone, Default)]
pub(crate) struct Allowance {
    /// At the most permissive level, nothing is asked.
    pub(crate) everything: bool,
    /// What stays inside the Agent's own folder is not asked about.
    pub(crate) contained: bool,
    /// What the Owner allowed once and for all.
    pub(crate) always: Vec<String>,
}

impl Allowance {
    /// True when the Agent itself would do this without asking.
    fn lets(&self, approval: &crate::Approval, action: &crate::ActionRequest) -> bool {
        if self.everything {
            return true;
        }
        if approval.critical {
            return false;
        }
        (self.contained && approval.contained)
            || self.always.contains(&crate::agent::covered_by_always(action))
    }
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
        (
            Self {
                running: HashMap::new(),
                reports,
                steps: 60,
                allowed: Arc::new(std::sync::RwLock::new(Allowance::default())),
            },
            arriving,
        )
    }

    /// What those at work for the Agent may do without asking, from now on.
    pub(crate) fn allow(&self, allowance: Allowance) {
        *self.allowed.write().expect("not poisoned") = allowance;
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
                 reaches you later as a notice. It knows nothing of your conversation and can \
                 ask nobody.",
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
        let allowed = self.allowed.clone();
        let work = tokio::spawn(async move {
            let working = work(&*provider, &host, &given, steps, &allowed);
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

/// Who is using the Computer, and when they last did. There is one screen:
/// the Agent and those working beside it take turns at it, and it is free
/// again a little after whoever had it stops.
static AT_THE_COMPUTER: std::sync::Mutex<Option<(String, Instant)>> = std::sync::Mutex::new(None);

/// How long after its last use the Computer is still its user's.
const KEPT_FOR: Duration = Duration::from_secs(40);

/// True when `who` has the Computer now: it was free, or already theirs.
pub(crate) fn take_the_computer(who: &str) -> bool {
    let mut at = AT_THE_COMPUTER.lock().expect("not poisoned");
    match &*at {
        Some((other, since)) if other != who && since.elapsed() < KEPT_FOR => false,
        _ => {
            *at = Some((who.to_owned(), Instant::now()));
            true
        }
    }
}

/// Works on `task` to its end; the text says how it ended and carries the report.
async fn work(
    provider: &impl Provider,
    host: &Host,
    task: &str,
    steps: usize,
    allowed: &std::sync::RwLock<Allowance>,
) -> String {
    let actions: Vec<ActionSpec> = host
        .specs()
        .into_iter()
        .filter(|action| !NOT_FOR_SUBAGENTS.contains(&action.name.as_str()))
        .collect();
    let mut conversation = vec![Message::from_owner(task)];
    for _ in 0..steps {
        // What the Computer's screen showed lately is shown to the model, as
        // it is to the Agent itself: without it a Sub-agent uses a Computer
        // it cannot see.
        let recent = conversation.len().saturating_sub(SHOWING_PICTURES);
        let images: Vec<(usize, std::path::PathBuf)> = conversation
            .iter()
            .enumerate()
            .skip(recent)
            .filter_map(|(place, message)| match message {
                Message::Notice(text) => Some((place, text)),
                _ => None,
            })
            .flat_map(|(place, text)| {
                crate::attachments::attachments(text)
                    .filter(|file| crate::attachments::is_showable_image(file))
                    .map(move |file| (place, file))
            })
            .collect();
        let request = Request {
            instructions: INSTRUCTIONS,
            conversation: &conversation,
            actions: &actions,
            images: &images,
            light: false,
        };
        let reply = match patiently(provider, request, host.busy_wait).await {
            Ok(reply) => reply,
            Err(error) => return format!("failed: {}", explain(&error, crate::Language::English)),
        };
        host.spent.lock().expect("not poisoned").extend(reply.usage.clone());
        conversation.push(Message::Agent {
            text: reply.text.clone(),
            actions: reply.actions.clone(),
            kept: reply.kept.clone(),
        });
        if reply.actions.is_empty() {
            return format!("finished. Its report:\n{}", reply.text);
        }
        let mut screen = None;
        for action in &reply.actions {
            let text = if NOT_FOR_SUBAGENTS.contains(&action.name.as_str()) {
                format!("There is no Action called {}.", action.name)
            } else if action.name.starts_with("computer") && !take_the_computer(task) {
                // One screen, one pair of hands at a time.
                "The Computer is being used by someone else working for the same Owner right \
                 now: two cannot share one screen. Do what you can another way (read_page, \
                 search_web), or try the Computer again in a minute."
                    .into()
            } else {
                match host.review(action) {
                    Review::Free => host.carry_out(action).await,
                    Review::Refuse(problem) => problem,
                    // What the Agent would do without asking, so does it.
                    Review::Ask(approval)
                        if allowed.read().expect("not poisoned").lets(&approval, action) =>
                    {
                        host.carry_out(action).await
                    }
                    Review::Ask(_) => "This needs the Owner's approval, which a sub-agent cannot \
                                       ask for. It was not carried out: say in your report what \
                                       would need doing."
                        .into(),
                }
            };
            // What the Computer's screen now shows, as the smaller picture
            // made beside it for a model to look at.
            if action.name.starts_with("computer")
                && let Some(picture) = crate::attachments::attachments(&text)
                    .find(|file| crate::attachments::is_showable_image(file))
            {
                let shown = crate::computer::shown_to_the_model(&picture);
                screen = Some(if shown.is_file() { shown } else { picture });
            }
            let text = clip(&text, OUTCOME_KEPT).to_owned();
            conversation.push(Message::Outcome { action: action.id.clone(), text });
        }
        // Shown once, after everything done on this step.
        if let Some(picture) = screen {
            conversation.push(Message::Notice(format!(
                "The Computer's screen now. Read what you need from it. If what it shows stops \
                 you (a page asking to verify that a person is there, an error, a sign-in), say \
                 in your report exactly what it shows:\n{}",
                crate::attached(&picture)
            )));
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
        light: false,
    };
    match patiently(provider, request, host.busy_wait).await {
        Ok(reply) => {
            host.spent.lock().expect("not poisoned").extend(reply.usage.clone());
            format!("ran out of steps. Its report so far:\n{}", reply.text)
        }
        Err(error) => {
            format!("ran out of steps, then failed: {}", explain(&error, crate::Language::English))
        }
    }
}
