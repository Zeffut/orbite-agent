//! How the Agent's work is shown to its Owner while it happens: one icon and
//! a few words per kind of Action, the same on every Channel.

use crate::ActionRequest;

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
    pub(crate) fn of(action: &ActionRequest) -> Self {
        let field = |name: &str| action.input[name].as_str().unwrap_or_default().trim().to_owned();
        let (icon, text) = match action.name.as_str() {
            "run_command" => ("💻", format!("Running: {}", field("command"))),
            "read_file" => ("📄", format!("Reading {}", field("path"))),
            "write_file" => ("✍️", format!("Writing {}", field("path"))),
            "edit_file" => ("✏️", format!("Editing {}", field("path"))),
            "search_web" => ("🔎", format!("Searching the web: {}", field("query"))),
            "read_page" => {
                // The site is what matters to a reader, not the whole address.
                let site = url::Url::parse(&field("url"))
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_else(|| field("url"));
                ("🌐", format!("Reading {site}"))
            }
            "remember" => ("🧠", format!("Remembering: {}", field("name"))),
            "forget" => ("🧠", format!("Forgetting: {}", field("name"))),
            "search_conversation" => ("🗂", format!("Looking back for: {}", field("query"))),
            "use_skill" => ("🧩", format!("Using the skill {}", field("name"))),
            "save_skill" => ("🧩", format!("Saving the skill {}", field("name"))),
            "remove_skill" => ("🧩", format!("Removing the skill {}", field("name"))),
            "delegate" => ("🤝", format!("Starting a sub-agent: {}", field("name"))),
            "list_subagents" => ("🤝", "Checking on sub-agents".to_owned()),
            "stop_subagent" => ("🤝", format!("Stopping the sub-agent {}", field("name"))),
            "schedule_routine" => ("⏰", format!("Scheduling: {}", field("name"))),
            "list_routines" => ("⏰", "Checking routines".to_owned()),
            "pause_routine" => ("⏰", format!("Pausing the routine {}", field("name"))),
            "resume_routine" => ("⏰", format!("Resuming the routine {}", field("name"))),
            "remove_routine" => ("⏰", format!("Removing the routine {}", field("name"))),
            "run_routine" => ("⏰", format!("Trying out the routine {}", field("name"))),
            "send_file" => {
                let path = field("path");
                let name = path.rsplit('/').next().unwrap_or_default().to_owned();
                ("📎", format!("Sending {name}"))
            }
            "set_quiet_hours" => ("🌙", "Setting quiet hours".to_owned()),
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
