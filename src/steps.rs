//! How the Agent's work is shown to its Owner while it happens: one icon and
//! a few words per kind of Action, the same on every Channel.

use crate::{ActionRequest, Language};

/// The longest text of a step.
const TEXT_LIMIT: usize = 80;

/// One thing the Agent is doing, as shown to its Owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub icon: &'static str,
    /// A few words, without the icon.
    pub text: String,
}

impl Step {
    /// How carrying out `action` is shown.
    pub(crate) fn of(action: &ActionRequest, language: Language) -> Self {
        let say = |english: &str, french: &str| language.say(english, french).to_owned();
        let field = |name: &str| action.input[name].as_str().unwrap_or_default().trim().to_owned();
        let (icon, text) = match action.name.as_str() {
            "run_command" => {
                ("💻", format!("{} {}", say("Running:", "Commande :"), field("command")))
            }
            "read_file" => ("📄", format!("{} {}", say("Reading", "Lecture de"), field("path"))),
            "write_file" => ("✍️", format!("{} {}", say("Writing", "Écriture de"), field("path"))),
            "edit_file" => {
                ("✏️", format!("{} {}", say("Editing", "Modification de"), field("path")))
            }
            "search_web" => (
                "🔎",
                format!(
                    "{} {}",
                    say("Searching the web:", "Recherche sur le web :"),
                    field("query")
                ),
            ),
            "read_page" => {
                // The site is what matters to a reader, not the whole address.
                let site = url::Url::parse(&field("url"))
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_else(|| field("url"));
                ("🌐", format!("{} {site}", say("Reading", "Lecture de")))
            }
            "remember" => {
                ("🧠", format!("{} {}", say("Remembering:", "Je retiens :"), field("name")))
            }
            "forget" => ("🧠", format!("{} {}", say("Forgetting:", "J'oublie :"), field("name"))),
            "search_conversation" => (
                "🗂",
                format!(
                    "{} {}",
                    say("Looking back for:", "Je recherche dans nos échanges :"),
                    field("query")
                ),
            ),
            "use_skill" => (
                "🧩",
                format!("{} {}", say("Using the skill", "J'applique la compétence"), field("name")),
            ),
            "save_skill" => (
                "🧩",
                format!(
                    "{} {}",
                    say("Saving the skill", "J'enregistre la compétence"),
                    field("name")
                ),
            ),
            "remove_skill" => (
                "🧩",
                format!(
                    "{} {}",
                    say("Removing the skill", "Je supprime la compétence"),
                    field("name")
                ),
            ),
            "delegate" => (
                "🤝",
                format!(
                    "{} {}",
                    say("Starting a sub-agent:", "Je lance un sous-agent :"),
                    field("name")
                ),
            ),
            "list_subagents" => ("🤝", say("Checking on sub-agents", "Je vérifie les sous-agents")),
            "stop_subagent" => (
                "🤝",
                format!(
                    "{} {}",
                    say("Stopping the sub-agent", "J'arrête le sous-agent"),
                    field("name")
                ),
            ),
            "schedule_routine" => {
                ("⏰", format!("{} {}", say("Scheduling:", "Je planifie :"), field("name")))
            }
            "list_routines" => ("⏰", say("Checking routines", "Je vérifie les routines")),
            "pause_routine" => (
                "⏰",
                format!(
                    "{} {}",
                    say("Pausing the routine", "Je mets en pause la routine"),
                    field("name")
                ),
            ),
            "resume_routine" => (
                "⏰",
                format!(
                    "{} {}",
                    say("Resuming the routine", "Je reprends la routine"),
                    field("name")
                ),
            ),
            "remove_routine" => (
                "⏰",
                format!(
                    "{} {}",
                    say("Removing the routine", "Je supprime la routine"),
                    field("name")
                ),
            ),
            "run_routine" => (
                "⏰",
                format!(
                    "{} {}",
                    say("Trying out the routine", "J'essaie la routine"),
                    field("name")
                ),
            ),
            "send_file" => {
                let path = field("path");
                let name = path.rsplit('/').next().unwrap_or_default().to_owned();
                ("📎", format!("{} {name}", say("Sending", "Envoi de")))
            }
            "set_quiet_hours" => {
                ("🌙", say("Setting quiet hours", "Je règle les heures de silence"))
            }
            other => ("⚙️", other.replace('_', " ")),
        };
        Self { icon, text: shortened(&text) }
    }
}

/// `text` on one line, cut with an ellipsis when too long to take in at a glance.
fn shortened(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= TEXT_LIMIT {
        return line;
    }
    let kept: String = line.chars().take(TEXT_LIMIT - 1).collect();
    format!("{}…", kept.trim_end())
}
