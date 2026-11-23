//! The Actions the Agent can carry out on the Host, and the single gate that
//! decides which of them need the Owner's Approval first.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::Language;
use crate::computer::Desk;
use crate::extensions::{Extensions, Unremovable};
use crate::mcp::Mcp;
use crate::memory::Memory;
use crate::secrets::Secrets;
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

const NO_EXTENSIONS: &str = "There is nowhere to keep Extensions.";

/// The rules for writing an Extension, given to the model when it is about
/// to write one rather than on every turn.
const WRITING_AN_EXTENSION: &str = "How to write an Extension. Call create_extension again with: name, \
description (one sentence for your Owner), hosts (the hosts it reaches, by name, such as api.example.com; \
none when it reaches nothing), secrets (the names of the secrets it sends them; none when it needs none) \
and script, the whole JavaScript program.\n\
The script defines two global functions.\n\
- describe() returns a list of { name, description, parameters }: parameters maps each parameter's name \
(ending in ? when it may be left out) to its description. Add asks: true to an action that changes \
something outside.\n\
- run(action, input) does the action called `action` and returns its result, a text or an object. It may \
be async, and throws an Error to say what went wrong.\n\
It has plain JavaScript and what Orbite lends, nothing else: no fetch, require, file or timer.\n\
- store.get(key), store.set(key, value), store.remove(key), store.keys(): data it keeps.\n\
- http.get(url, { headers }), http.post(url, body, { headers }), also put, patch and delete: they return \
{ status, body, json() } and reach only the hosts listed.\n\
- To send a secret (see list_secrets), write {{NAME}} in an address, a header or a body: Orbite fills it in \
on the way, and the script never holds its value.\n\
Its Actions are then called name_action. Try each one before telling your Owner it works.";

const NO_COMPUTER: &str = "You have no Computer on this machine.";

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
    /// What was used by those who answer to the Agent and keep no account
    /// themselves: Sub-agents and Routines. The Agent takes it from there.
    pub(crate) spent: std::sync::Mutex<Vec<crate::Usage>>,
    /// Where commands run and relative paths start.
    pub(crate) workspace: PathBuf,
    /// A command still running after this long is stopped.
    pub(crate) command_timeout: Duration,
    /// Where the Agent keeps its Memory, when it has somewhere to.
    pub(crate) memory: Option<Memory>,
    /// Where the Agent keeps its Skills, when it has somewhere to.
    pub(crate) skills: Option<Skills>,
    pub(crate) web: Web,
    /// How long a busy Provider is first waited for.
    pub(crate) busy_wait: Duration,
    /// The secrets the Agent can use, when it has any.
    pub(crate) secrets: Option<Secrets>,
    /// The language Approvals are worded in.
    pub(crate) language: Language,
    /// The Extensions of this installation, when it has somewhere for them.
    pub(crate) extensions: Option<Arc<Extensions>>,
    /// The MCP servers of this installation, when it has any.
    pub(crate) mcp: Option<Arc<Mcp>>,
    /// The Agent's Computer, when it has one.
    pub(crate) computer: Option<Desk>,
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
struct CreateExtension {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    secrets: Vec<String>,
    #[serde(default)]
    script: String,
}

#[derive(Deserialize)]
struct NamedExtension {
    name: String,
}

/// Something done on the Computer.
#[derive(Deserialize)]
#[serde(tag = "gesture", rename_all = "lowercase")]
enum Gesture {
    Look,
    Open {
        url: String,
        #[serde(default)]
        purpose: String,
    },
    Click {
        x: u16,
        y: u16,
        #[serde(default)]
        double: bool,
    },
    Type {
        text: String,
    },
    Key {
        keys: String,
    },
    Scroll {
        notches: i32,
    },
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
    ListSecrets,
    CreateExtension(CreateExtension),
    ReadExtension(NamedExtension),
    RemoveExtension(NamedExtension),
    Computer(Gesture),
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
        if self.secrets.is_some() {
            specs.push(spec(
                "list_secrets",
                "Lists the names of the secrets your Owner entered on the machine. You never see \
                 their values: a command uses one as $NAME.",
                json!({}),
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
        if self.computer.is_some() {
            let whole =
                |description: &str| json!({ "type": "integer", "description": description });
            specs.push(spec(
                "computer_open",
                "Opens an address in the browser of your own Computer: a real screen that \
                 you see through pictures and act on with mouse and keyboard, for sites that need \
                 signing in or a person's gestures. Your Owner is asked first. Before anything that \
                 pays, sends or deletes, and when a site asks for a code or a person, ask your Owner.",
                json!({
                    "url": text("The address, starting with https://."),
                    "purpose": text("What you are going there to do, in a few words, for your Owner."),
                }),
            ));
            specs.push(spec(
                "computer_look",
                "Shows you the Computer's screen as it is now.",
                json!({}),
            ));
            specs.push(ActionSpec {
                name: "computer_click".into(),
                description: "Clicks at a point of the Computer's screen, counted in pixels from its top left corner, as in the last picture.".into(),
                input: json!({
                    "type": "object",
                    "properties": { "x": whole("Across, from 0 to 852."), "y": whole("Down, from 0 to 532."), "double": { "type": "boolean", "description": "True to click twice." } },
                    "required": ["x", "y"],
                }),
            });
            specs.push(spec(
                "computer_type",
                "Types a text on the Computer's keyboard, where the cursor is: click the field first. \
                 To type a secret (see list_secrets), write its name as {{NAME}}: its value is typed \
                 without you seeing it, after your Owner agreed.",
                json!({ "text": text("What to type.") }),
            ));
            specs.push(spec(
                "computer_key",
                "Presses a key or a combination on the Computer's keyboard.",
                json!({ "keys": text("Such as Return, Tab, Escape, BackSpace, Page_Down, ctrl+l, ctrl+a, alt+Left.") }),
            ));
            specs.push(ActionSpec {
                name: "computer_scroll".into(),
                description: "Turns the mouse wheel where the pointer is.".into(),
                input: json!({
                    "type": "object",
                    "properties": { "notches": whole("How far: positive for down, negative for up; 5 is about a screen.") },
                    "required": ["notches"],
                }),
            });
        }
        if self.extensions.is_some() {
            specs.push(ActionSpec {
                name: "create_extension".into(),
                description: "Writes yourself an Extension: a small JavaScript program that adds Actions \
                    you can then use, such as a connector to a service your Owner uses often; or \
                    replaces one of the same name, to repair it. Your Owner is shown it and asked \
                    first. Call it with only `name` to be given the rules for writing one."
                    .into(),
                input: json!({
                    "type": "object",
                    "properties": {
                        "name": text("A few lowercase words joined by dashes, such as \"weather\"."),
                        "description": { "type": "string" },
                        "hosts": { "type": "array", "items": { "type": "string" } },
                        "secrets": { "type": "array", "items": { "type": "string" } },
                        "script": { "type": "string" },
                    },
                    "required": ["name"],
                }),
            });
            specs.push(spec(
                "read_extension",
                "Shows an Extension's script and state, to repair or improve it.",
                json!({ "name": text("The Extension's name.") }),
            ));
            specs.push(spec(
                "remove_extension",
                "Removes an Extension and everything it kept. Needs the Owner's approval.",
                json!({ "name": text("The Extension's name.") }),
            ));
        }
        specs
    }

    /// Does `gesture` on the Computer; the Outcome ends with the picture of
    /// the screen once it has settled.
    async fn on_the_computer(&self, gesture: Gesture) -> String {
        let Some(desk) = &self.computer else { return NO_COMPUTER.into() };
        // Reached here with the Owner's say where one was needed: it holds
        // for as long as the Agent keeps working there.
        desk.taken();
        // What the screen showed before, to know when an address has opened.
        let before = match &gesture {
            Gesture::Open { .. } => desk.glance().ok(),
            _ => None,
        };
        let (done, patience) = match gesture {
            Gesture::Look => (Ok("This is the screen.".to_owned()), Duration::from_secs(2)),
            Gesture::Open { url, .. } => {
                (desk.open(&url).map(|()| format!("Opened {url}.")), Duration::from_secs(25))
            }
            Gesture::Click { x, y, double } => (
                // Aimed at on the picture the model was shown.
                desk.click(x, y, double).map(|()| format!("Clicked at {x}, {y}.")),
                Duration::from_secs(6),
            ),
            Gesture::Type { text } => {
                // A secret is typed by its name; its value goes to the keyboard only.
                let lent: Vec<(String, String)> = text
                    .split("{{")
                    .skip(1)
                    .filter_map(|after| after.split_once("}}"))
                    // Orbite's own keys are not the Agent's to type.
                    .filter(|(name, _)| !name.starts_with("ORBITE_"))
                    .filter_map(|(name, _)| {
                        Some((name.to_owned(), self.secrets.as_ref()?.get(name).ok()??))
                    })
                    .collect();
                let typed = match crate::extensions::filled(&text, &lent) {
                    Ok(typed) => desk.type_text(&typed).map(|()| "Typed.".to_owned()),
                    Err(_) => Err(std::io::Error::other(
                        "a secret named there has not been entered: your Owner gives one by sending \
                         /secret NAME in the conversation, which you never see",
                    )),
                };
                (typed, Duration::from_secs(4))
            }
            Gesture::Key { keys } => {
                (desk.key(&keys).map(|()| format!("Pressed {keys}.")), Duration::from_secs(6))
            }
            Gesture::Scroll { notches } => {
                (desk.scroll(notches).map(|()| "Scrolled.".to_owned()), Duration::from_secs(3))
            }
        };
        let done = match done {
            Ok(done) => done,
            Err(error) => return format!("That could not be done on the Computer: {error}."),
        };
        // A browser that is starting shows nothing for a moment, then its
        // page: after opening, the screen has to stay still for longer.
        let calm = if before.is_some() { 3 } else { 2 };
        match desk.picture(patience, calm, before.as_deref()).await {
            Ok(picture) => format!("{done}\n{}", crate::attached(&picture)),
            Err(error) => format!("{done} The screen could not be pictured: {error}."),
        }
    }

    /// The Actions the Extensions add, as told to the model, without any
    /// that would take the name of one in `taken`: an Extension never
    /// stands in for an Action of Orbite itself.
    pub(crate) fn extension_specs(&self, taken: &[ActionSpec]) -> Vec<ActionSpec> {
        let mut specs =
            self.extensions.as_ref().map(|extensions| extensions.specs()).unwrap_or_default();
        specs.retain(|spec| taken.iter().all(|other| other.name != spec.name));
        specs
    }

    /// True when `command` is pointed at the folder Orbite keeps everything
    /// in, or at one that holds it, rather than at the Agent's own folder or
    /// elsewhere on the machine: looking there, even without naming a file,
    /// shows what is sealed and who is let in.
    fn looks_around_orbites_home(&self, command: &str) -> bool {
        let Some(home) = self.workspace.parent() else { return false };
        let read = command.replace(['\'', '"', '\\'], "");
        read.split(|c: char| c.is_whitespace() || "=;|&()<>".contains(c)).any(|word| {
            // Up from where commands run, which is the Agent's folder.
            if word == ".."
                || word.starts_with("../")
                || word.contains("/../")
                || word.ends_with("/..")
            {
                return true;
            }
            // The account's own folder is Orbite's.
            if word.starts_with('~') || word.to_lowercase().starts_with("$home") {
                return true;
            }
            // Named whole: Orbite's folder, one above it, or inside it but
            // outside the Agent's.
            let whole = Path::new(word.trim_end_matches(['*', '?']));
            whole.is_absolute()
                && (home.starts_with(whole)
                    || (whole.starts_with(home) && !whole.starts_with(&self.workspace)))
        })
    }

    /// True when `address` is where Orbite's own web interface is served or
    /// reached.
    fn is_the_interface(&self, address: &reqwest::Url) -> bool {
        let Some(config) = self.workspace.parent().and_then(|home| crate::Config::load(home).ok())
        else {
            return false;
        };
        let asked = (address.host_str().unwrap_or_default(), address.port_or_known_default());
        let served = config.interface.as_deref().and_then(|served| {
            let served: std::net::SocketAddr = served.parse().ok()?;
            let here =
                ["localhost", "127.0.0.1", "[::1]", &served.ip().to_string()].contains(&asked.0);
            Some(here && asked.1 == Some(served.port()))
        });
        let reached = config.interface_reached_at.as_deref().and_then(|reached| {
            let reached = reqwest::Url::parse(reached).ok()?;
            Some(
                reached.host_str().is_some_and(|name| name.eq_ignore_ascii_case(asked.0))
                    && reached.port_or_known_default() == asked.1,
            )
        });
        served == Some(true) || reached == Some(true)
    }

    /// The gate every Action passes through before it is carried out.
    pub(crate) fn review(&self, request: &ActionRequest) -> Review {
        if !is_own(&request.name) {
            let asked = self
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.review(request))
                .or_else(|| self.mcp.as_ref().and_then(|mcp| mcp.review(request)));
            if let Some(asked) = asked {
                return asked.map_or(Review::Free, Review::Ask);
            }
        }
        let action = match understand(request) {
            Ok(action) => action,
            Err(problem) => return Review::Refuse(problem),
        };
        match action {
            Action::RunCommand(RunCommand { command }) => {
                // As the shell will read it: quotes inside a name hide nothing.
                let reveals = looks_secret(&command) || looks_secret(&command.replace(['\'', '"'], ""));
                if only_looks(&command)
                    && !reveals
                    && !names_orbites_own(&command)
                    && !self.looks_around_orbites_home(&command)
                {
                    return Review::Free;
                }
                let mut detail = command;
                if reveals {
                    detail.push_str("\n\n");
                    detail.push_str(self.language.say(
                        "This may show a secret to the AI provider.",
                        "Cela peut montrer un secret au fournisseur d'IA.",
                    ));
                }
                Review::Ask(Approval {
                    summary: self.language.say("Run a command", "Lancer une commande").into(),
                    critical: reveals || is_critical(&detail),
                    detail,
                    // Orbite has no way to undo what a command did.
                    irreversible: true,
                    ..Approval::default()
                })
            }
            Action::ReadFile(ReadFile { path }) => {
                if !looks_secret(&path) {
                    return Review::Free;
                }
                Review::Ask(Approval {
                    summary: self.language.say("Read a file", "Lire un fichier").into(),
                    critical: true,
                    detail: format!(
                        "{path}\n\n{}",
                        self.language.say(
                            "This may show a secret to the AI provider.",
                            "Cela peut montrer un secret au fournisseur d'IA."
                        )
                    ),
                    irreversible: true,
                    ..Approval::default()
                })
            }
            Action::WriteFile(WriteFile { path, content }) => {
                let replaces = self.locate(&path).exists();
                let summary = if replaces {
                    self.language.say("Replace a file", "Remplacer un fichier")
                } else {
                    self.language.say("Create a file", "Créer un fichier")
                };
                // What decides what the Agent can reach is not an ordinary file.
                let own = self.is_orbites_own(&path);
                Review::Ask(Approval {
                    summary: summary.into(),
                    contained: self.contains(&path) && !own,
                    detail: format!("{path}\n\n{}", preview(&content)),
                    irreversible: replaces,
                    critical: own,
                })
            }
            Action::ListSecrets => Review::Free,
            Action::SearchWeb(_) => Review::Free,
            Action::ReadPage(page) => match web_address(&page.url) {
                Err(problem) => Review::Refuse(problem),
                Ok(url) if !crate::web::is_local(&url) => Review::Free,
                Ok(_) => Review::Ask(Approval {
                    summary: self
                        .language
                        .say(
                            "Read an address on your own network",
                            "Lire une adresse de ton réseau",
                        )
                        .into(),
                    detail: format!(
                        "{}\n\n{}",
                        page.url,
                        self.language.say(
                            "What it returns will be shown to the AI provider.",
                            "Ce qu'elle renvoie sera montré au fournisseur d'IA."
                        )
                    ),
                    irreversible: false,
                    ..Approval::default()
                }),
            },
            Action::UseSkill(_) => Review::Free,
            Action::SaveSkill(skill) => {
                let Some(skills) = &self.skills else { return Review::Refuse(NO_SKILLS.into()) };
                if let Err(error) = crate::slug::slug(&skill.name) {
                    return Review::Refuse(format!("Could not save the skill: {error}."));
                }
                let replaces = skills.has(&skill.name);
                let summary = if replaces {
                    self.language.say("Replace a skill", "Remplacer une compétence")
                } else {
                    self.language.say("Save a skill", "Enregistrer une compétence")
                };
                Review::Ask(Approval {
                    contained: true,
                    summary: summary.into(),
                    detail: format!(
                        "{}\n\n{}\n\n{}",
                        skill.name,
                        skill.description,
                        preview(&skill.instructions)
                    ),
                    irreversible: replaces,
                    ..Approval::default()
                })
            }
            Action::RemoveSkill(skill) => match self.skill(&skill.name) {
                Err(problem) => Review::Refuse(problem),
                Ok(found) => Review::Ask(Approval {
                    contained: true,
                    summary: self.language.say("Remove a skill", "Supprimer une compétence").into(),
                    detail: format!("{}\n\n{}", found.name, found.description),
                    irreversible: true,
                    ..Approval::default()
                }),
            },
            // Asked for the rules, not yet for an Extension.
            Action::CreateExtension(wanted) if wanted.script.trim().is_empty() => Review::Free,
            Action::CreateExtension(wanted) => {
                let Some(extensions) = &self.extensions else {
                    return Review::Refuse(NO_EXTENSIONS.into());
                };
                let adds = match extensions.check(&wanted.name, &wanted.script) {
                    Ok(adds) => adds,
                    Err(problem) => return Review::Refuse(problem),
                };
                let replaces = extensions.has(&wanted.name);
                let summary = if replaces {
                    self.language.say("Replace an extension", "Remplacer une extension")
                } else {
                    self.language.say("Create an extension", "Créer une extension")
                };
                let reaches = if wanted.hosts.is_empty() {
                    self.language
                        .say(
                            "Reaches nothing outside this machine.",
                            "N'atteint rien hors de cette machine.",
                        )
                        .to_owned()
                } else {
                    format!(
                        "{} {}",
                        self.language.say("Reaches:", "Atteint :"),
                        wanted.hosts.join(", ")
                    )
                };
                let reaches = if wanted.secrets.is_empty() {
                    reaches
                } else {
                    format!(
                        "{reaches}\n{} {}",
                        self.language.say("Sends them the secrets:", "Leur envoie les secrets :"),
                        wanted.secrets.join(", ")
                    )
                };
                // Code the Agent wrote for itself is shown to its Owner at
                // every Permission Level, and never allowed once and for all.
                Review::Ask(Approval {
                    summary: summary.into(),
                    detail: format!(
                        "{}: {}\n{reaches}\n{} {}\n\n{}",
                        wanted.name,
                        wanted.description,
                        self.language.say("Adds:", "Ajoute :"),
                        adds.join(", "),
                        preview(&wanted.script)
                    ),
                    irreversible: replaces,
                    critical: true,
                    contained: true,
                })
            }
            Action::ReadExtension(_) => Review::Free,
            Action::RemoveExtension(named) => match &self.extensions {
                None => Review::Refuse(NO_EXTENSIONS.into()),
                Some(extensions) => match extensions.read(&named.name) {
                    None => Review::Refuse(format!("There is no Extension called {}.", named.name)),
                    Some(_) => Review::Ask(Approval {
                        summary: self
                            .language
                            .say("Remove an extension", "Supprimer une extension")
                            .into(),
                        detail: format!(
                            "{}\n\n{}",
                            named.name,
                            self.language.say(
                                "Everything it kept is removed with it.",
                                "Tout ce qu'elle gardait est supprimé avec elle."
                            )
                        ),
                        irreversible: true,
                        contained: true,
                        ..Approval::default()
                    }),
                },
            },
            Action::Computer(_) if self.computer.is_none() => Review::Refuse(NO_COMPUTER.into()),
            // This computer's mouse and keyboard do whatever its Owner can:
            // taking them is asked, and again once the Agent has let go.
            Action::Computer(gesture)
                if self.computer.as_ref().is_some_and(|desk| desk.asks_before_taking()) =>
            {
                // What is refused on any Computer is refused here first, and
                // what is asked on any Computer is said in this one request:
                // nothing gets through under a more general question.
                let about = match &gesture {
                    Gesture::Open { url, purpose } => match web_address(url) {
                        Err(problem) => return Review::Refuse(problem),
                        Ok(address) if self.is_the_interface(&address) => {
                            return Review::Refuse(
                                "That is the page your Owner talks to you in: it is theirs alone \
                                 to open."
                                    .into(),
                            );
                        }
                        Ok(address) => format!(
                            "\n\n{}\n{}",
                            address.host_str().unwrap_or_default(),
                            purpose.trim()
                        ),
                    },
                    Gesture::Type { text } if text.contains("{{") => format!(
                        "\n\n{text}\n{}",
                        self.language.say(
                            "The value of that secret is typed in place of its name.",
                            "La valeur de ce secret est tapée à la place de son nom.",
                        )
                    ),
                    _ => String::new(),
                };
                Review::Ask(Approval {
                    summary: self
                        .language
                        .say("Use this computer", "Utiliser cet ordinateur")
                        .into(),
                    detail: format!(
                        "{}{about}",
                        self.language.say(
                            "Your agent takes this computer's mouse and keyboard, in your session, with \
                             the accounts you are signed in to. It keeps them for ten minutes, then asks \
                             again.",
                            "Ton agent prend la souris et le clavier de cet ordinateur, dans ta session, \
                             avec les comptes où tu es connecté. Il les garde dix minutes, puis redemande.",
                        )
                    ),
                    // Asked every time, at every level but the free one, and
                    // never allowed once and for all.
                    critical: true,
                    ..Approval::default()
                })
            }
            Action::Computer(Gesture::Open { url, purpose }) => match web_address(&url) {
                Err(problem) => Review::Refuse(problem),
                // The page its Owner talks to it in is not the Agent's to
                // open: there it would answer its own requests.
                Ok(address) if self.is_the_interface(&address) => Review::Refuse(
                    "That is the page your Owner talks to you in: it is theirs alone to open."
                        .into(),
                ),
                // Going to a site is asked; what is done there afterwards is not.
                Ok(address) => Review::Ask(Approval {
                    summary: self.language.say("Use the computer on a site", "Utiliser l'ordinateur sur un site").into(),
                    detail: format!("{}\n\n{}", address.host_str().unwrap_or_default(), purpose.trim()),
                    ..Approval::default()
                }),
            },
            // Typing a secret is how an account is entered: asked every time.
            Action::Computer(Gesture::Type { text }) if text.contains("{{") => Review::Ask(Approval {
                summary: self.language.say("Type a secret on the computer", "Taper un secret sur l'ordinateur").into(),
                detail: format!(
                    "{}\n\n{}",
                    text,
                    self.language.say(
                        "Its value is typed in place of its name. If the field shows what is typed, the \
                         picture of the screen will show it to the AI provider.",
                        "Sa valeur est tapée à la place de son nom. Si le champ affiche ce qui est tapé, la \
                         photo de l'écran la montrera au fournisseur d'IA."
                    )
                ),
                critical: true,
                irreversible: true,
                ..Approval::default()
            }),
            Action::Computer(_) => Review::Free,
            // Memory is the Agent's own, and its Owner can read and undo all of it.
            Action::Remember(_) | Action::Forget(_) => Review::Free,
            // Asked before the file is looked at: whether a passage is there
            // already tells what a file holds.
            Action::EditFile(edit) if looks_secret(&edit.path) => Review::Ask(Approval {
                summary: self.language.say("Edit a file", "Modifier un fichier").into(),
                critical: true,
                detail: format!(
                    "{}\n\n{}",
                    edit.path,
                    self.language.say(
                        "This may show a secret to the AI provider.",
                        "Cela peut montrer un secret au fournisseur d'IA."
                    )
                ),
                irreversible: true,
                ..Approval::default()
            }),
            Action::EditFile(edit) => match self.edited(&edit) {
                Err(problem) => Review::Refuse(problem),
                Ok(_) => Review::Ask(Approval {
                    summary: self.language.say("Edit a file", "Modifier un fichier").into(),
                    critical: self.is_orbites_own(&edit.path),
                    contained: self.contains(&edit.path) && !self.is_orbites_own(&edit.path),
                    detail: format!(
                        "{}\n\n{}\n{}\n\n{}\n{}",
                        edit.path,
                        self.language.say("Replace:", "Remplacer :"),
                        preview(&edit.old),
                        self.language.say("With:", "Par :"),
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
        let outcome = self.do_it(request).await;
        // Whatever an Action brings back, no secret goes on to the model.
        match &self.secrets {
            Some(secrets) => secrets.hide(&outcome),
            None => outcome,
        }
    }

    async fn do_it(&self, request: &ActionRequest) -> String {
        if !is_own(&request.name) {
            if let Some(extensions) = &self.extensions
                && let Some(outcome) = extensions.carry_out(request, self.secrets.as_ref()).await
            {
                return outcome;
            }
            if let Some(mcp) = &self.mcp
                && let Some(outcome) = mcp.carry_out(request).await
            {
                return outcome;
            }
        }
        match understand(request) {
            Err(problem) => problem,
            Ok(Action::RunCommand(run)) => self.run(&run.command).await,
            Ok(Action::ReadFile(read)) => self.read(&read.path),
            Ok(Action::WriteFile(write)) => self.write(&write.path, &write.content),
            Ok(Action::ListSecrets) => {
                match self.secrets.as_ref().map(Secrets::names_for_the_agent) {
                    Some(Ok(names)) if names.is_empty() => {
                        "No secret has been entered. Your Owner gives one by sending /secret NAME \
                     in the conversation, which you never see."
                            .into()
                    }
                    Some(Ok(names)) => names.join("\n"),
                    Some(Err(error)) => format!("The secrets could not be read: {error}."),
                    None => "There is nowhere to keep secrets.".into(),
                }
            }
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
            Ok(Action::CreateExtension(wanted)) if wanted.script.trim().is_empty() => {
                WRITING_AN_EXTENSION.into()
            }
            Ok(Action::CreateExtension(wanted)) => match &self.extensions {
                None => NO_EXTENSIONS.into(),
                Some(extensions) => {
                    match extensions.check(&wanted.name, &wanted.script).and_then(|adds| {
                        extensions
                            .write(
                                &wanted.name,
                                &wanted.description,
                                &wanted.hosts,
                                &wanted.secrets,
                                &wanted.script,
                            )
                            .map(|()| adds)
                            .map_err(|error| format!("Could not keep the Extension: {error}."))
                    }) {
                        Ok(adds) => format!(
                            "The Extension {} is in place and adds: {}. Try each of them now, before telling \
                         your Owner it works.",
                            wanted.name,
                            adds.join(", ")
                        ),
                        Err(problem) => problem,
                    }
                }
            },
            Ok(Action::ReadExtension(named)) => self
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.read(&named.name))
                .unwrap_or_else(|| format!("There is no Extension called {}.", named.name)),
            Ok(Action::RemoveExtension(named)) => match &self.extensions {
                None => NO_EXTENSIONS.into(),
                Some(extensions) => match extensions.remove(&named.name) {
                    Ok(()) => {
                        format!("Removed the Extension {} and everything it kept.", named.name)
                    }
                    Err(Unremovable::Official) => format!(
                        "{} comes with Orbite and cannot be removed; your Owner can turn it off with \
                         /extensions off {}.",
                        named.name, named.name
                    ),
                    Err(Unremovable::Unknown) => {
                        format!("There is no Extension called {}.", named.name)
                    }
                },
            },
            Ok(Action::Computer(gesture)) => self.on_the_computer(gesture).await,
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

    /// True when `path` lies inside the Agent's workspace, wherever its
    /// `..` parts lead.
    fn contains(&self, path: &str) -> bool {
        let mut resolved = PathBuf::new();
        for part in self.locate(path).components() {
            match part {
                std::path::Component::ParentDir => {
                    resolved.pop();
                }
                std::path::Component::CurDir => {}
                other => resolved.push(other),
            }
        }
        resolved.starts_with(&self.workspace)
    }

    /// True when `path` is one of the files that decide what the Agent can
    /// reach or holds its secrets: its Extensions, its MCP servers, its
    /// settings. Writing there goes through the Actions made for it, or is
    /// asked about at every Permission Level.
    fn is_orbites_own(&self, path: &str) -> bool {
        let mut resolved = PathBuf::new();
        for part in self.locate(path).components() {
            match part {
                std::path::Component::ParentDir => {
                    resolved.pop();
                }
                std::path::Component::CurDir => {}
                other => resolved.push(other),
            }
        }
        let Some(home) = self.workspace.parent() else { return false };
        let Ok(inside) = resolved.strip_prefix(home) else { return false };
        let first =
            inside.components().next().map(|part| part.as_os_str().to_string_lossy().into_owned());
        matches!(
            first.as_deref(),
            Some(
                "extensions"
                    | "mcp.json"
                    | "config.json"
                    | "secrets.key"
                    | "secrets.sealed"
                    | "tryouts.json"
                    // Who is let into the Conversation, and what they see.
                    | "interface.db"
                    | "interface.db-wal"
                    | "interface.db-shm"
                    | "interface-files"
            )
        )
    }

    /// `path` as the Host sees it: `~` is the home folder, and a relative
    /// path starts in the workspace.
    pub(crate) fn locate(&self, path: &str) -> PathBuf {
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
        // What Orbite itself was started with (its Provider's key, its bot's
        // token) is not for the commands it runs.
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("ORBITE_") {
                shell.env_remove(name);
            }
        }
        // A secret reaches a command only when the command names it.
        if let Some(secrets) = &self.secrets {
            shell.envs(secrets.named_in(command));
        }
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

/// True for the name of an Action of Orbite itself.
fn is_own(name: &str) -> bool {
    let named =
        ActionRequest { id: String::new(), name: name.to_owned(), input: serde_json::Value::Null };
    !understand(&named).is_err_and(|problem| problem.starts_with(NO_SUCH_ACTION))
}

const NO_SUCH_ACTION: &str = "There is no Action called";

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
        "list_secrets" => Ok(Action::ListSecrets),
        "create_extension" => input(request).map(Action::CreateExtension),
        "read_extension" => input(request).map(Action::ReadExtension),
        "remove_extension" => input(request).map(Action::RemoveExtension),
        "save_skill" => input(request).map(Action::SaveSkill),
        "remove_skill" => input(request).map(Action::RemoveSkill),
        other => match other.strip_prefix("computer_") {
            Some(gesture @ ("look" | "open" | "click" | "type" | "key" | "scroll")) => {
                let mut input = match &request.input {
                    serde_json::Value::Object(given) => given.clone(),
                    _ => serde_json::Map::new(),
                };
                input.insert("gesture".into(), gesture.into());
                serde_json::from_value(input.into())
                    .map(Action::Computer)
                    .map_err(|error| format!("The input of {other} is not valid: {error}."))
            }
            _ => Err(format!("{NO_SUCH_ACTION} {other}.")),
        },
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
    // What is written between single quotes reaches the program as it is,
    // the shell working nothing out of it: a pattern for find or grep. It is
    // set aside for the shell's part, and kept for the program's.
    // One line only: a second one is a second command, whatever is read
    // as quoted here. The shell and this reading must not differ.
    if command.contains(['\n', '\r']) {
        return false;
    }
    let mut outside = String::new();
    let mut unquoted = String::new();
    let mut quoted = false;
    for letter in command.chars() {
        if letter == '\'' {
            quoted = !quoted;
            // Something stands there, whatever it is.
            if quoted {
                outside.push('_');
            }
            continue;
        }
        if !quoted {
            outside.push(letter);
        }
        // A space inside quotes does not end the word the program is given.
        unquoted.push(if quoted && letter.is_whitespace() { '_' } else { letter });
    }
    if quoted {
        return false;
    }
    // Nothing the shell would work out by itself after this look: what a
    // pattern or ~ ends up naming cannot be known here.
    if outside.contains(|c| "#;&><$`(){}\\\n\r!*?[]~\"".contains(c)) {
        return false;
    }
    // Where the secrets given to a command can be read back from, however
    // the name is written.
    let plain = command.replace('\'', "");
    if ["/proc", "/dev/fd", "/dev/std"].iter().any(|place| plain.contains(place)) {
        return false;
    }
    // A | between quotes is not a pipe: such a command is not taken apart here.
    if outside.matches('|').count() != unquoted.matches('|').count() {
        return false;
    }
    unquoted.split('|').all(|stage| {
        let mut words = stage.split_whitespace();
        let Some(program) = words.next() else {
            return false;
        };
        let options: Vec<&str> = words.collect();
        // One-letter options written together, such as -us.
        let together = |letter: char| {
            options.iter().any(|word| {
                word.starts_with('-') && !word.starts_with("--") && word.contains(letter)
            })
        };
        let harmless = match program {
            "hostname" => options.is_empty(),
            // Sets the clock: -s, --set and whatever shortens or joins them.
            "date" => !together('s') && !options.iter().any(|word| word.starts_with("--s")),
            // Writes a compiled list of kinds beside the one it is given.
            "file" => !together('C') && !options.iter().any(|word| word.starts_with("--c")),
            // With e among its plain letters, shows what each program was started with.
            "ps" => !options.iter().any(|word| !word.starts_with('-') && word.contains('e')),
            _ => true,
        };
        LOOKING.contains(&program)
            && options.iter().all(|word| !CHANGING.contains(word))
            && harmless
    })
}

/// True when `command` could do damage no Permission Level should let
/// through unasked: taking administrator rights, wiping, shutting down,
/// running what was just downloaded. A list of known dangers, not a proof.
fn is_critical(command: &str) -> bool {
    if names_orbites_own(command) {
        return true;
    }
    is_dangerous(command)
}

/// True when `command` names what says who Orbite listens to, or what it
/// keeps sealed or of the Conversation: whatever program is pointed at it,
/// and were it only to look.
fn names_orbites_own(command: &str) -> bool {
    const ORBITES_OWN: &[&str] = &[
        "interface.db",
        "interface-files",
        "secrets.sealed",
        "secrets.key",
        "orbite.db",
        // Where the Agent is lent a desktop.
        "orbite-desk",
        // What opens the web interface to the Internet.
        "interface public",
        "interface_public",
    ];
    // As the shell will read it: quotes and backslashes inside a name hide
    // nothing.
    let command = command.to_lowercase().replace(['\'', '"', '\\'], "");
    ORBITES_OWN.iter().any(|file| command.contains(file))
}

fn is_dangerous(command: &str) -> bool {
    const DANGEROUS: &[&str] = &[
        // Orbite's own command changes its Providers, settings and secrets.
        "orbite",
        "sudo",
        "su",
        "doas",
        "pkexec",
        "shutdown",
        "reboot",
        "poweroff",
        "halt",
        "passwd",
        "useradd",
        "userdel",
        "usermod",
        "visudo",
        "fdisk",
        "parted",
        "wipefs",
        "iptables",
        "ufw",
        "crontab",
        "systemctl",
        "service",
        "chown",
        "mount",
        "umount",
    ];
    let command = command.to_lowercase();
    let words: Vec<&str> = command
        .split(|c: char| c.is_whitespace() || ";|&()".contains(c))
        .filter(|word| !word.is_empty())
        .collect();
    let runs = |program: &str| {
        words.iter().any(|word| *word == program || word.ends_with(&format!("/{program}")))
    };
    let outside =
        |word: &&str| word.starts_with('/') || word.starts_with('~') || word.starts_with("$home");
    DANGEROUS.iter().any(|program| runs(program))
        || names_orbites_own(&command)
        || words.iter().any(|word| word.starts_with("mkfs"))
        || (runs("rm") && words.iter().any(outside))
        || (runs("dd") && command.contains("of=/dev"))
        || (runs("chmod") && words.iter().any(outside))
        || command.contains("/dev/sd")
        || command.contains(":(){")
        // What was just downloaded, run without being read.
        || ["| sh", "|sh", "| bash", "|bash", "| zsh", "|zsh"].iter().any(|pipe| command.contains(pipe))
}

/// True when `text` names something that is probably a secret.
pub(crate) fn looks_secret(text: &str) -> bool {
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
