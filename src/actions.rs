//! The Actions the Agent can carry out on the Host, and the single gate that
//! decides which of them need the Owner's Approval first.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::memory::Memory;
use crate::skills::{Skill, Skills};
use crate::web::Web;
use crate::{ActionRequest, ActionSpec, Approval};

/// How much of a command's output, per stream, is passed on to the model.
const OUTPUT_KEPT: usize = 16_000;
/// How much of a file is passed on to the model.
const FILE_KEPT: usize = 100_000;
/// How much of a text is shown to the Owner in an Approval.
const PREVIEW_KEPT: usize = 1_500;

const NO_SKILLS: &str = "There is nowhere to keep skills.";

/// What the gate decided about an Action.
pub(crate) enum Review {
    /// It only looks at things: carry it out.
    Free,
    /// It changes or reveals something: the Owner decides.
    Ask(Approval),
    /// It cannot be carried out at all; the text tells the model why.
    Refuse(String),
}

/// The Agent's hands on the machine it runs on.
pub(crate) struct Host {
    /// Where commands run and relative paths start.
    pub(crate) workspace: PathBuf,
    /// A command still running after this long is stopped.
    pub(crate) command_timeout: Duration,
    /// Where the Agent keeps its Memory, when it has somewhere to.
    pub(crate) memory: Option<Memory>,
    /// Where the Agent keeps its Skills, when it has somewhere to.
    pub(crate) skills: Option<Skills>,
    pub(crate) web: Web,
}

#[derive(Deserialize)]
struct ReadPage {
    url: String,
}

#[derive(Deserialize)]
struct SearchWeb {
    query: String,
}

#[derive(Deserialize)]
struct NamedSkill {
    name: String,
}

#[derive(Deserialize)]
struct SaveSkill {
    name: String,
    description: String,
    instructions: String,
}

#[derive(Deserialize)]
struct Remember {
    name: String,
    text: String,
}

#[derive(Deserialize)]
struct Forget {
    name: String,
}

#[derive(Deserialize)]
struct RunCommand {
    command: String,
}

#[derive(Deserialize)]
struct ReadFile {
    path: String,
}

#[derive(Deserialize)]
struct WriteFile {
    path: String,
    content: String,
}

#[derive(Deserialize)]
struct EditFile {
    path: String,
    old: String,
    new: String,
}

enum Action {
    RunCommand(RunCommand),
    ReadFile(ReadFile),
    WriteFile(WriteFile),
    EditFile(EditFile),
    Remember(Remember),
    Forget(Forget),
    UseSkill(NamedSkill),
    SaveSkill(SaveSkill),
    RemoveSkill(NamedSkill),
    ReadPage(ReadPage),
    SearchWeb(SearchWeb),
}

impl Host {
    /// The Actions on offer, as told to the model.
    pub(crate) fn specs(&self) -> Vec<ActionSpec> {
        let text = |description: &str| json!({ "type": "string", "description": description });
        let spec = |name: &str, description: &str, properties: serde_json::Value| {
            let required: Vec<&String> = properties.as_object().unwrap().keys().collect();
            ActionSpec {
                name: name.into(),
                description: description.into(),
                input: json!({ "type": "object", "properties": properties, "required": required }),
            }
        };
        let mut specs = vec![
            spec(
                "run_command",
                "Runs a shell command on the Owner's machine, in your workspace folder, and \
                 returns what it printed. Commands that only look at things run at once; any \
                 other command first needs the Owner's approval, so prefer several simple \
                 commands to one clever one.",
                json!({ "command": text("The command, as typed in a shell.") }),
            ),
            spec(
                "read_file",
                "Returns the text of a file.",
                json!({ "path": text("Absolute, or relative to your workspace folder.") }),
            ),
            spec(
                "write_file",
                "Creates a file, or replaces all of an existing one. Needs the Owner's approval.",
                json!({
                    "path": text("Absolute, or relative to your workspace folder."),
                    "content": text("The whole new content of the file."),
                }),
            ),
            spec(
                "edit_file",
                "Replaces one passage of a file. The passage must appear exactly once in it. \
                 Needs the Owner's approval.",
                json!({
                    "path": text("Absolute, or relative to your workspace folder."),
                    "old": text("The exact passage to replace."),
                    "new": text("What to put in its place."),
                }),
            ),
            spec(
                "search_web",
                "Searches the web and returns the first results: title, address and excerpt. \
                 Follow up with read_page on the results worth reading.",
                json!({ "query": text("What to search for, as typed in a search engine.") }),
            ),
            spec(
                "read_page",
                "Returns the readable content of a web page, or the text at a web address.",
                json!({ "url": text("The full address, starting with http:// or https://.") }),
            ),
        ];
        if self.memory.is_some() {
            specs.push(spec(
                "remember",
                "Keeps something about your Owner for all later conversations. Using a name \
                 already in your memory replaces that entry, which is how you correct it.",
                json!({
                    "name": text("A few words naming the entry, such as \"home\" or \"coffee order\"."),
                    "text": text("What to remember, written so it makes sense on its own."),
                }),
            ));
            specs.push(spec(
                "forget",
                "Deletes an entry of your memory.",
                json!({ "name": text("The entry's name, as listed in your memory.") }),
            ));
        }
        if self.skills.is_some() {
            specs.push(spec(
                "use_skill",
                "Gives you the instructions of one of your skills. Use it before doing anything \
                 a skill covers.",
                json!({ "name": text("The skill's name, as listed in your skills.") }),
            ));
            specs.push(spec(
                "save_skill",
                "Keeps a way of doing something as a skill for later conversations, or improves \
                 an existing one saved under the same name. Needs the Owner's approval.",
                json!({
                    "name": text("A few lowercase words joined by dashes, such as \"weekly-report\"."),
                    "description": text(
                        "One or two sentences: what the skill does, and when to use it. This is \
                         all you will see of it until you use it."
                    ),
                    "instructions": text("The steps to follow, in Markdown."),
                }),
            ));
            specs.push(spec(
                "remove_skill",
                "Deletes a skill and its files. Needs the Owner's approval.",
                json!({ "name": text("The skill's name, as listed in your skills.") }),
            ));
        }
        specs
    }

    /// The gate every Action passes through before it is carried out.
    pub(crate) fn review(&self, request: &ActionRequest) -> Review {
        let action = match understand(request) {
            Ok(action) => action,
            Err(problem) => return Review::Refuse(problem),
        };
        match action {
            Action::RunCommand(RunCommand { command }) => {
                let reveals = looks_secret(&command);
                if only_looks(&command) && !reveals {
                    return Review::Free;
                }
                let mut detail = command;
                if reveals {
                    detail.push_str("\n\nThis may show a secret to the AI provider.");
                }
                Review::Ask(Approval {
                    summary: "Run a command".into(),
                    detail,
                    // Orbite has no way to undo what a command did.
                    irreversible: true,
                })
            }
            Action::ReadFile(ReadFile { path }) => {
                if !looks_secret(&path) {
                    return Review::Free;
                }
                Review::Ask(Approval {
                    summary: "Read a file".into(),
                    detail: format!("{path}\n\nThis may show a secret to the AI provider."),
                    irreversible: true,
                })
            }
            Action::WriteFile(WriteFile { path, content }) => {
                let replaces = self.locate(&path).exists();
                let summary = if replaces { "Replace a file" } else { "Create a file" };
                Review::Ask(Approval {
                    summary: summary.into(),
                    detail: format!("{path}\n\n{}", preview(&content)),
                    irreversible: replaces,
                })
            }
            Action::SearchWeb(_) => Review::Free,
            Action::ReadPage(page) => match web_address(&page.url) {
                Err(problem) => Review::Refuse(problem),
                Ok(url) if !crate::web::is_local(&url) => Review::Free,
                Ok(_) => Review::Ask(Approval {
                    summary: "Read an address on your own network".into(),
                    detail: format!(
                        "{}\n\nWhat it returns will be shown to the AI provider.",
                        page.url
                    ),
                    irreversible: false,
                }),
            },
            Action::UseSkill(_) => Review::Free,
            Action::SaveSkill(skill) => {
                let Some(skills) = &self.skills else { return Review::Refuse(NO_SKILLS.into()) };
                if let Err(error) = crate::slug::slug(&skill.name) {
                    return Review::Refuse(format!("Could not save the skill: {error}."));
                }
                let replaces = skills.has(&skill.name);
                let summary = if replaces { "Replace a skill" } else { "Save a skill" };
                Review::Ask(Approval {
                    summary: summary.into(),
                    detail: format!(
                        "{}\n\n{}\n\n{}",
                        skill.name,
                        skill.description,
                        preview(&skill.instructions)
                    ),
                    irreversible: replaces,
                })
            }
            Action::RemoveSkill(skill) => match self.skill(&skill.name) {
                Err(problem) => Review::Refuse(problem),
                Ok(found) => Review::Ask(Approval {
                    summary: "Remove a skill".into(),
                    detail: format!("{}\n\n{}", found.name, found.description),
                    irreversible: true,
                }),
            },
            // Memory is the Agent's own, and its Owner can read and undo all of it.
            Action::Remember(_) | Action::Forget(_) => Review::Free,
            Action::EditFile(edit) => match self.edited(&edit) {
                Err(problem) => Review::Refuse(problem),
                Ok(_) => Review::Ask(Approval {
                    summary: "Edit a file".into(),
                    detail: format!(
                        "{}\n\nReplace:\n{}\n\nWith:\n{}",
                        edit.path,
                        preview(&edit.old),
                        preview(&edit.new)
                    ),
                    // The replaced passage stays in the Conversation.
                    irreversible: false,
                }),
            },
        }
    }

    /// Carries out an Action the gate let through; the text is its Outcome.
    pub(crate) async fn carry_out(&self, request: &ActionRequest) -> String {
        match understand(request) {
            Err(problem) => problem,
            Ok(Action::RunCommand(run)) => self.run(&run.command).await,
            Ok(Action::ReadFile(read)) => self.read(&read.path),
            Ok(Action::WriteFile(write)) => self.write(&write.path, &write.content),
            Ok(Action::SearchWeb(search)) => self.web.search(&search.query).await,
            Ok(Action::ReadPage(page)) => match web_address(&page.url) {
                Err(problem) => problem,
                Ok(url) => self.web.read(&url).await,
            },
            Ok(Action::Remember(entry)) => self.remember(&entry),
            Ok(Action::UseSkill(skill)) => match self.skill(&skill.name) {
                Err(problem) => problem,
                Ok(skill) => format!(
                    "{}\n\n[This skill's files are in {}]",
                    skill.instructions,
                    skill.folder.display()
                ),
            },
            Ok(Action::SaveSkill(skill)) => match &self.skills {
                None => NO_SKILLS.into(),
                Some(skills) => {
                    match skills.save(&skill.name, &skill.description, &skill.instructions) {
                        Ok(name) => format!("Saved the skill \"{name}\"."),
                        Err(error) => format!("Could not save the skill: {error}."),
                    }
                }
            },
            Ok(Action::RemoveSkill(skill)) => match &self.skills {
                None => NO_SKILLS.into(),
                Some(skills) => match skills.remove(&skill.name) {
                    Ok(true) => format!("Removed the skill \"{}\".", skill.name),
                    Ok(false) => format!("There is no skill called \"{}\".", skill.name),
                    Err(error) => format!("Could not remove the skill: {error}."),
                },
            },
            Ok(Action::Forget(entry)) => self.forget(&entry.name),
            Ok(Action::EditFile(edit)) => match self.edited(&edit) {
                Err(problem) => problem,
                Ok(content) => match std::fs::write(self.locate(&edit.path), content) {
                    Ok(()) => format!("Edited {}.", edit.path),
                    Err(error) => format!("Could not edit {}: {error}.", edit.path),
                },
            },
        }
    }

    /// The Skill called `name`, or what to tell the model when there is none.
    fn skill(&self, name: &str) -> Result<Skill, String> {
        let Some(skills) = &self.skills else { return Err(NO_SKILLS.into()) };
        skills.find(name).ok_or_else(|| {
            let names: Vec<String> = skills.all().into_iter().map(|skill| skill.name).collect();
            format!("There is no skill called \"{name}\". The skills are: {}.", names.join(", "))
        })
    }

    fn remember(&self, entry: &Remember) -> String {
        let Some(memory) = &self.memory else { return "There is no memory to keep it in.".into() };
        match memory.remember(&entry.name, &entry.text) {
            Ok(name) => format!("Remembered as \"{name}\"."),
            Err(error) => format!("Could not remember it: {error}."),
        }
    }

    fn forget(&self, name: &str) -> String {
        let Some(memory) = &self.memory else { return "There is no memory.".into() };
        match memory.forget(name) {
            Ok(true) => format!("Forgot \"{name}\"."),
            Ok(false) => {
                let names: Vec<String> =
                    memory.entries().into_iter().map(|(name, _)| name).collect();
                format!(
                    "Nothing is remembered as \"{name}\". The entries are: {}.",
                    names.join(", ")
                )
            }
            Err(error) => format!("Could not forget it: {error}."),
        }
    }

    /// `path` as the Host sees it: `~` is the home folder, and a relative
    /// path starts in the workspace.
    fn locate(&self, path: &str) -> PathBuf {
        if let Some(rest) = path.strip_prefix("~/")
            && let Some(home) = std::env::var_os("HOME")
        {
            return Path::new(&home).join(rest);
        }
        self.workspace.join(path)
    }

    fn read(&self, path: &str) -> String {
        let bytes = match std::fs::read(self.locate(path)) {
            Ok(bytes) => bytes,
            Err(error) => return format!("Could not read {path}: {error}."),
        };
        let Ok(text) = String::from_utf8(bytes) else {
            return format!("{path} is not a text file.");
        };
        match cut(&text, FILE_KEPT) {
            (kept, 0) => kept.to_owned(),
            (kept, missing) => format!("{kept}\n[file cut: {missing} more bytes]"),
        }
    }

    fn write(&self, path: &str, content: &str) -> String {
        let target = self.locate(path);
        let written = target
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&target, content));
        match written {
            Ok(()) => format!("Wrote {} bytes to {path}.", content.len()),
            Err(error) => format!("Could not write {path}: {error}."),
        }
    }

    /// The file's content once `edit` is applied, or why it cannot be.
    fn edited(&self, edit: &EditFile) -> Result<String, String> {
        let path = &edit.path;
        let content = std::fs::read_to_string(self.locate(path))
            .map_err(|error| format!("Could not read {path}: {error}."))?;
        match content.matches(&edit.old).count() {
            1 => Ok(content.replacen(&edit.old, &edit.new, 1)),
            0 => Err(format!("The passage to replace does not appear in {path}; nothing changed.")),
            times => Err(format!(
                "The passage to replace appears {times} times in {path}; nothing changed. \
                 Give a longer passage that appears once."
            )),
        }
    }

    async fn run(&self, command: &str) -> String {
        if let Err(error) = std::fs::create_dir_all(&self.workspace) {
            return format!("Could not use the workspace folder: {error}.");
        }
        let mut shell = Command::new("sh");
        shell
            .arg("-c")
            .arg(command)
            .current_dir(&self.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Its own process group, so that everything the command started
            // can be stopped with it.
            .process_group(0)
            .kill_on_drop(true);
        let mut child = match shell.spawn() {
            Ok(child) => child,
            Err(error) => return format!("Could not start the command: {error}."),
        };
        let group = child.id();
        // Should the Agent be stopped while the command runs, everything it
        // started goes with it.
        let mut running = StopGroup(group);
        let printed = tokio::spawn(keep_start(child.stdout.take().expect("stdout is piped")));
        let complained = tokio::spawn(keep_start(child.stderr.take().expect("stderr is piped")));

        let status = tokio::time::timeout(self.command_timeout, child.wait()).await;
        if status.is_err()
            && let Some(group) = group
        {
            // SAFETY: `killpg` only sends a signal; `group` is the process
            // group created for this command just above.
            unsafe { libc::killpg(group as libc::pid_t, libc::SIGKILL) };
            let _ = child.wait().await;
        }

        // It ended by itself: what it deliberately left running stays.
        if status.is_ok() {
            running.0 = None;
        }

        let mut outcome = String::new();
        for (label, stream) in [("", printed), ("[stderr]\n", complained)] {
            let (kept, missing) = stream.await.unwrap_or_default();
            if kept.is_empty() {
                continue;
            }
            if !outcome.is_empty() {
                outcome.push('\n');
            }
            outcome.push_str(label);
            outcome.push_str(String::from_utf8_lossy(&kept).trim_end());
            if missing > 0 {
                outcome.push_str(&format!("\n[output cut: {missing} more bytes]"));
            }
        }
        match status {
            Err(_) => outcome.push_str(&format!(
                "\n[The command was stopped after {:?} without finishing.]",
                self.command_timeout
            )),
            Ok(Ok(status)) if !status.success() => match status.code() {
                Some(code) => outcome.push_str(&format!("\n[exit code {code}]")),
                None => outcome.push_str("\n[The command was killed.]"),
            },
            Ok(Ok(_)) if outcome.is_empty() => outcome.push_str("(no output)"),
            Ok(Ok(_)) => {}
            Ok(Err(error)) => outcome.push_str(&format!("\n[Could not wait for it: {error}]")),
        }
        outcome.trim_start().to_owned()
    }
}

/// The Action a request stands for, or what to tell the model is wrong.
fn understand(request: &ActionRequest) -> Result<Action, String> {
    fn input<T: DeserializeOwned>(request: &ActionRequest) -> Result<T, String> {
        serde_json::from_value(request.input.clone())
            .map_err(|error| format!("The input of {} is not valid: {error}.", request.name))
    }
    match request.name.as_str() {
        "run_command" => input(request).map(Action::RunCommand),
        "read_file" => input(request).map(Action::ReadFile),
        "write_file" => input(request).map(Action::WriteFile),
        "edit_file" => input(request).map(Action::EditFile),
        "remember" => input(request).map(Action::Remember),
        "forget" => input(request).map(Action::Forget),
        "use_skill" => input(request).map(Action::UseSkill),
        "read_page" => input(request).map(Action::ReadPage),
        "search_web" => input(request).map(Action::SearchWeb),
        "save_skill" => input(request).map(Action::SaveSkill),
        "remove_skill" => input(request).map(Action::RemoveSkill),
        other => Err(format!("There is no Action called {other}.")),
    }
}

/// `address` as a web address, or what to tell the model is wrong with it.
fn web_address(address: &str) -> Result<url::Url, String> {
    match url::Url::parse(address.trim()) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.host().is_some() => Ok(url),
        _ => Err(format!(
            "\"{address}\" is not a web address: it must start with http:// or https://."
        )),
    }
}

/// True when `command` certainly changes nothing: a pipeline of programs that
/// only look, with nothing the shell could turn into something else. Anything
/// not recognised is assumed to change something.
fn only_looks(command: &str) -> bool {
    const LOOKING: &[&str] = &[
        "cat", "cut", "date", "df", "du", "echo", "file", "find", "free", "grep", "head",
        "hostname", "id", "ls", "lsblk", "lscpu", "nproc", "ps", "pwd", "stat", "tail", "tr",
        "true", "uname", "uptime", "wc", "which", "whoami",
    ];
    // Options that make a looking program write, run or set something.
    const CHANGING: &[&str] = &[
        "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fls", "-fprint", "-fprint0", "-fprintf",
        "-s", "--set",
    ];
    if command.contains(|c| ";&><$`(){}\\\n\r!".contains(c)) {
        return false;
    }
    command.split('|').all(|stage| {
        let mut words = stage.split_whitespace();
        let Some(program) = words.next() else {
            return false;
        };
        LOOKING.contains(&program)
            && words.all(|word| !CHANGING.contains(&word))
            && (program != "hostname" || stage.split_whitespace().count() == 1)
    })
}

/// True when `text` names something that is probably a secret.
fn looks_secret(text: &str) -> bool {
    const SECRET: &[&str] = &[
        ".ssh",
        ".gnupg",
        ".aws",
        ".netrc",
        ".env",
        "secret",
        "password",
        "shadow",
        "token",
        "credential",
        "id_rsa",
        "id_ed25519",
        "private",
        "keychain",
        "wallet",
    ];
    let text = text.to_lowercase();
    SECRET.iter().any(|mark| text.contains(mark))
}

/// The start of `text` that fits in `limit` bytes, and how many bytes are left out.
fn cut(text: &str, limit: usize) -> (&str, usize) {
    if text.len() <= limit {
        return (text, 0);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], text.len() - end)
}

fn preview(text: &str) -> String {
    match cut(text, PREVIEW_KEPT) {
        (kept, 0) => kept.to_owned(),
        (kept, missing) => format!("{kept}\n[… {missing} more bytes]"),
    }
}

/// Reads `stream` to its end, keeping its first bytes and counting the rest,
/// so that a command printing without end neither blocks nor fills memory.
async fn keep_start(mut stream: impl AsyncRead + Unpin) -> (Vec<u8>, usize) {
    let mut kept = Vec::new();
    let mut missing = 0;
    let mut buffer = [0u8; 8192];
    while let Ok(read) = stream.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        let room = OUTPUT_KEPT - kept.len();
        kept.extend_from_slice(&buffer[..read.min(room)]);
        missing += read.saturating_sub(room);
    }
    (kept, missing)
}

/// Stops a command's whole process group when dropped.
struct StopGroup(Option<u32>);

impl Drop for StopGroup {
    fn drop(&mut self) {
        if let Some(group) = self.0 {
            // SAFETY: `killpg` only sends a signal; `group` is the process
            // group created for the command this guard was made for.
            unsafe { libc::killpg(group as libc::pid_t, libc::SIGKILL) };
        }
    }
}
