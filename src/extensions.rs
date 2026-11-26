//! Extensions: scripts that add Actions to the Agent, run in a sandbox where
//! they reach nothing but what Orbite lends them: a store of their own and
//! the hosts they were granted, over the web or, for a service that speaks
//! something else (mail, say), over a connection to one of its ports.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rquickjs::{Context, Function, Runtime};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::secrets::Secrets;
use crate::slug::slug;
use crate::{ActionRequest, ActionSpec, Approval};

/// The Official Extensions, shipped in the program: (name, manifest, script).
const OFFICIAL: &[(&str, &str, &str)] = &[(
    "tasks",
    include_str!("../extensions/tasks/extension.json"),
    include_str!("../extensions/tasks/main.js"),
)];

/// The most memory one run of an Extension may take.
const MEMORY_LIMIT: usize = 32 * 1024 * 1024;

/// The longest text an Extension is given back from a host.
const BODY_LIMIT: usize = 200_000;

/// The largest value an Extension keeps under one key, as written.
const VALUE_LIMIT: usize = 1_000_000;

/// The most an Extension keeps in all.
const STORE_LIMIT: usize = 10_000_000;

/// The longest name of an Action: what every Provider accepts.
const NAME_LIMIT: usize = 64;

/// How many connections one run of an Extension holds at once.
const CONNECTIONS_AT_ONCE: usize = 4;

/// How many connections one run opens in all.
const CONNECTIONS_IN_ALL: u32 = 12;

/// The longest mark a read is asked to go up to.
const MARK_LIMIT: usize = 1_000;

/// How long nothing must have come for what has come to be all of it,
/// beyond a few times what the service takes to reach; and at the most.
const QUIET: Duration = Duration::from_millis(200);
const QUIET_AT_MOST: Duration = Duration::from_secs(2);

/// How long a connection is waited on for what it has to say, unasked.
const READ_WAIT: Duration = Duration::from_secs(10);

/// How many redirections in a row an Extension's request follows.
const REDIRECTIONS_FOLLOWED: usize = 6;

/// The reason kept for an Extension its Owner turned off.
pub(crate) const BY_THE_OWNER: &str = "by the Owner";

/// What Orbite lends, as an Extension sees it.
const LENT: &str = r#"
const store = Object.freeze({
  get: key => { const kept = __store_get(String(key)); return kept === "" ? null : JSON.parse(kept); },
  set: (key, value) => {
    if (value === undefined || value === null) __store_remove(String(key));
    else { const refused = __store_set(String(key), JSON.stringify(value)); if (refused) throw new Error(refused); }
  },
  remove: key => { __store_remove(String(key)); },
  keys: () => JSON.parse(__store_keys()),
});
const http = (() => {
  const request = (method, url, options) => {
    options = options || {};
    let body = options.body;
    if (body !== undefined && typeof body !== "string") body = JSON.stringify(body);
    const answer = JSON.parse(__http(JSON.stringify({ method, url: String(url), headers: options.headers || {}, body })));
    if (answer.error) throw new Error(answer.error);
    answer.json = () => JSON.parse(answer.body);
    return answer;
  };
  return Object.freeze({
    get: (url, options) => request("GET", url, options),
    delete: (url, options) => request("DELETE", url, options),
    post: (url, body, options) => request("POST", url, { ...options, body }),
    put: (url, body, options) => request("PUT", url, { ...options, body }),
    patch: (url, body, options) => request("PATCH", url, { ...options, body }),
  });
})();
const net = (() => {
  const ask = request => {
    const answer = JSON.parse(__net(JSON.stringify(request)));
    if (answer.error) throw new Error(answer.error);
    return answer;
  };
  return Object.freeze({
    connect: (host, port, options) => {
      const id = ask({ op: "connect", host: String(host), port: Number(port), tls: !(options && options.tls === false) }).id;
      return Object.freeze({
        write: data => { ask({ op: "write", id, data: String(data) }); },
        read: options => ask({ op: "read", id, until: options && options.until, timeout: options && options.timeout }).data,
        startTls: () => { ask({ op: "starttls", id }); },
        close: () => { ask({ op: "close", id }); },
      });
    },
  });
})();
"#;

#[derive(Deserialize, Default)]
struct Manifest {
    #[serde(default)]
    description: String,
    /// The same in French, for an Official Extension.
    description_fr: Option<String>,
    /// The hosts the Extension may reach, by name.
    #[serde(default)]
    hosts: Vec<String>,
    /// The secrets it may send to those hosts, by name.
    #[serde(default)]
    secrets: Vec<String>,
}

/// One Extension of this installation.
pub(crate) struct Extension {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) description_fr: Option<String>,
    hosts: Vec<String>,
    secrets: Vec<String>,
    script: String,
    /// True for an Extension shipped with Orbite.
    pub(crate) official: bool,
    /// Why it is turned off, when it is.
    pub(crate) off: Option<String>,
}

/// One Action an Extension adds, as the Extension describes it.
#[derive(Clone)]
struct Described {
    name: String,
    description: String,
    input: Value,
    asks: bool,
    critical: bool,
}

/// How a run of an Extension went wrong.
enum Failure {
    /// The script refused what it was asked: the Extension is sound.
    Threw(String),
    /// The Extension itself does not work.
    Broke(String),
}

pub(crate) struct Extensions {
    folder: PathBuf,
    pub(crate) time_limit: Duration,
    http: reqwest::Client,
    /// What each Extension describes, by name and script.
    described: Mutex<HashMap<(String, String), Vec<Described>>>,
    /// Extensions turned off since the Agent was last told.
    reports: Mutex<Vec<String>>,
}

impl Extensions {
    pub(crate) fn kept_in(folder: PathBuf) -> Self {
        Self {
            folder,
            time_limit: Duration::from_secs(20),
            http: reqwest::Client::builder()
                // A redirection could lead to a host that was not granted.
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
            described: Mutex::default(),
            reports: Mutex::default(),
        }
    }

    /// Every Extension of this installation, the Official ones first.
    pub(crate) fn all(&self) -> Vec<Extension> {
        let off = |name: &str| fs::read_to_string(self.folder.join(name).join("off")).ok();
        let mut all: Vec<Extension> = OFFICIAL
            .iter()
            .map(|(name, manifest, script)| {
                let manifest: Manifest = serde_json::from_str(manifest).unwrap_or_default();
                Extension {
                    name: (*name).to_owned(),
                    description: manifest.description,
                    description_fr: manifest.description_fr,
                    hosts: manifest.hosts,
                    secrets: manifest.secrets,
                    script: (*script).to_owned(),
                    official: true,
                    off: off(name),
                }
            })
            .collect();
        let mut names: Vec<String> = fs::read_dir(&self.folder)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.is_ascii() && slug(name).is_ok_and(|safe| &safe == name))
            .filter(|name| OFFICIAL.iter().all(|(official, ..)| official != name))
            .collect();
        names.sort();
        for name in names {
            let folder = self.folder.join(&name);
            let Ok(manifest) = fs::read_to_string(folder.join("extension.json")) else { continue };
            let manifest: Manifest = serde_json::from_str(&manifest).unwrap_or_default();
            all.push(Extension {
                description: manifest.description,
                description_fr: manifest.description_fr,
                hosts: manifest.hosts,
                secrets: manifest.secrets,
                script: fs::read_to_string(folder.join("main.js")).unwrap_or_default(),
                official: false,
                off: off(&name),
                name,
            });
        }
        all
    }

    /// The Actions the Extensions add, as told to the model. An Extension
    /// that cannot say what it adds is turned off.
    pub(crate) fn specs(&self) -> Vec<ActionSpec> {
        let mut specs = Vec::new();
        for extension in self.all().iter().filter(|extension| extension.off.is_none()) {
            for action in self.describe(extension).unwrap_or_default() {
                let name = format!("{}_{}", prefix(&extension.name), action.name);
                // One put there by hand that would clash, or that no
                // Provider would take, is left out rather than failing every turn.
                if name.len() > NAME_LIMIT
                    || specs.iter().any(|other: &ActionSpec| other.name == name)
                {
                    continue;
                }
                specs.push(ActionSpec {
                    name,
                    description: action.description,
                    input: action.input,
                });
            }
        }
        specs
    }

    /// What `extension` adds; None, and it is turned off, when it cannot say.
    fn describe(&self, extension: &Extension) -> Option<Vec<Described>> {
        let key = (extension.name.clone(), extension.script.clone());
        if let Some(known) = self.described.lock().unwrap().get(&key) {
            return Some(known.clone());
        }
        match self.described(extension) {
            Ok(described) => {
                self.described.lock().unwrap().insert(key, described.clone());
                Some(described)
            }
            Err(problem) => {
                self.turn_off(&extension.name, &format!("it could not be loaded: {problem}"));
                None
            }
        }
    }

    /// What `extension` says it adds, or why it cannot be loaded.
    fn described(&self, extension: &Extension) -> Result<Vec<Described>, String> {
        match self.run(extension, "JSON.stringify(describe())".into(), None) {
            Ok(said) => understand(&said),
            Err(Failure::Threw(problem) | Failure::Broke(problem)) => Err(problem),
        }
    }

    /// The Actions an Extension called `name` running `script` would add, or
    /// what is wrong with it. Nothing is kept.
    pub(crate) fn check(&self, name: &str, script: &str) -> Result<Vec<String>, String> {
        if slug(name).ok().as_deref() != Some(name) || !name.is_ascii() {
            return Err(format!(
                "\"{name}\" cannot name an Extension: use a few lowercase words joined by dashes."
            ));
        }
        if OFFICIAL.iter().any(|(official, ..)| *official == name) {
            return Err(format!("{name} comes with Orbite: choose another name."));
        }
        let extension = Extension {
            name: name.to_owned(),
            description: String::new(),
            description_fr: None,
            hosts: Vec::new(),
            secrets: Vec::new(),
            script: script.to_owned(),
            official: false,
            off: None,
        };
        let described = self.described(&extension).map_err(|problem| {
            format!(
                "The script does not load: {problem}. It must define the global functions \
                 describe(), returning a list of {{ name, description, parameters }}, and \
                 run(action, input); and check(), which tries it for real."
            )
        })?;
        if described.is_empty() {
            return Err("describe() returns no action: the Extension would add nothing.".into());
        }
        let adds: Vec<String> =
            described.iter().map(|action| format!("{}_{}", prefix(name), action.name)).collect();
        if let Some(long) = adds.iter().find(|action| action.len() > NAME_LIMIT) {
            return Err(format!(
                "{long} is too long a name for an Action: {NAME_LIMIT} letters at most, the Extension's name included."
            ));
        }
        // Two Extensions never add an Action of the same name.
        let taken: Vec<String> = self
            .all()
            .iter()
            .filter(|other| other.name != name && other.off.is_none())
            .flat_map(|other| {
                let prefix = prefix(&other.name);
                self.describe(other)
                    .unwrap_or_default()
                    .into_iter()
                    .map(move |action| format!("{prefix}_{}", action.name))
            })
            .collect();
        if let Some(same) = adds.iter().find(|action| taken.contains(action)) {
            return Err(format!(
                "Another Extension already adds an Action called {same}: choose another name."
            ));
        }
        Ok(adds)
    }

    /// True when the installation has an Extension called `name`.
    pub(crate) fn has(&self, name: &str) -> bool {
        self.all().iter().any(|extension| extension.name == name)
    }

    /// Keeps an Extension, in place of the one of the same name if any; what
    /// that one kept stays, and it is turned on.
    pub(crate) fn write(
        &self,
        name: &str,
        description: &str,
        hosts: &[String],
        secrets: &[String],
        script: &str,
    ) -> std::io::Result<()> {
        let folder = self.folder.join(name);
        fs::create_dir_all(&folder)?;
        let manifest = json!({ "description": description, "hosts": hosts, "secrets": secrets });
        fs::write(folder.join("extension.json"), serde_json::to_string_pretty(&manifest)? + "\n")?;
        fs::write(folder.join("main.js"), script)?;
        match fs::remove_file(folder.join("off")) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// An Extension as its author needs to see it to repair it.
    pub(crate) fn read(&self, name: &str) -> Option<String> {
        let extension = self.all().into_iter().find(|extension| extension.name == name)?;
        let state = match &extension.off {
            None => "It is on.".to_owned(),
            Some(reason) => format!("It is turned off: {reason}."),
        };
        Some(format!(
            "{name}: {}\nReaches: {}\n{state}\n\n{}",
            extension.description,
            listed(&extension.hosts),
            extension.script
        ))
    }

    /// The Extension and Action a request is for, when it is for one.
    fn find(&self, request: &ActionRequest) -> Option<(Extension, Described)> {
        self.all().into_iter().filter(|extension| extension.off.is_none()).find_map(|extension| {
            let action = request.name.strip_prefix(&prefix(&extension.name))?.strip_prefix('_')?;
            let described =
                self.describe(&extension)?.into_iter().find(|known| known.name == action)?;
            Some((extension, described))
        })
    }

    /// What the Owner is asked before `request`, when it is for an Extension:
    /// None inside when the Action is free to run.
    pub(crate) fn review(&self, request: &ActionRequest) -> Option<Option<Approval>> {
        let (extension, action) = self.find(request)?;
        Some((action.asks || action.critical).then(|| Approval {
            summary: format!("{} ({})", one_sentence(&action.description), extension.name),
            detail: serde_json::to_string_pretty(&request.input).unwrap_or_default(),
            irreversible: true,
            critical: action.critical,
            contained: false,
        }))
    }

    /// Carries out `request` when it is for an Extension; the text is its Outcome.
    pub(crate) async fn carry_out(
        self: &Arc<Self>,
        request: &ActionRequest,
        secrets: Option<&Secrets>,
    ) -> Option<String> {
        let (extension, action) = self.find(request)?;
        // The secrets it was granted are filled in on the way out: the
        // script names them and never holds them.
        let lent: Vec<(String, String)> = extension
            .secrets
            .iter()
            // Orbite's own keys are not the Agent's to hand out.
            .filter(|name| !name.starts_with("ORBITE_"))
            .filter_map(|name| Some((name.clone(), secrets?.get(name).ok()??)))
            .collect();
        let call = format!(
            "globalThis.__settled = null;
             Promise.resolve().then(() => run({}, {})).then(
               value => {{ __settled = {{ ok: value === undefined || value === null ? 'Done.'
                 : typeof value === 'string' ? value : JSON.stringify(value, null, 1) }}; }},
               error => {{ __settled = {{ error: String((error && error.message) || error) }}; }});",
            json!(action.name),
            request.input
        );
        let name = extension.name.clone();
        // What the Agent wrote for itself is its own to mend: its Owner is
        // not left with something that does not work.
        let mend = if extension.official {
            ""
        } else {
            "\nIf this is the Extension's own fault and not what you asked of it (the service \
             changed, a case it did not foresee), do not leave it there: read it with \
             read_extension, repair it, create it again, and do what your Owner asked."
        };
        let runner = Arc::clone(self);
        let handle = tokio::runtime::Handle::current();
        let ran =
            tokio::task::spawn_blocking(move || runner.run(&extension, call, Some((handle, lent))))
                .await;
        Some(match ran {
            Ok(Ok(settled)) => match serde_json::from_str::<Value>(&settled) {
                Ok(Value::Object(settled)) if settled.contains_key("ok") => {
                    settled["ok"].as_str().unwrap_or_default().to_owned()
                }
                Ok(Value::Object(settled)) => match settled["error"].as_str().unwrap_or("?") {
                    problem if problem.contains("out of memory") => {
                        self.broke(&name, "it took more memory than an Extension may")
                    }
                    problem => format!("The Extension {name} could not do it: {problem}{mend}"),
                },
                _ => format!("The Extension {name} did not finish what it started."),
            },
            Ok(Err(Failure::Threw(problem))) => {
                format!("The Extension {name} could not do it: {problem}{mend}")
            }
            Ok(Err(Failure::Broke(problem))) => self.broke(&name, &problem),
            Err(_) => self.broke(&name, "it crashed"),
        })
    }

    /// Tries the Extension called `name` for real, as its own `check()`
    /// says to: what it found, or what went wrong. None when the script
    /// has no check of its own.
    pub(crate) async fn tried(
        self: &Arc<Self>,
        name: &str,
        secrets: Option<&Secrets>,
    ) -> Option<Result<String, String>> {
        let extension = self.all().into_iter().find(|extension| extension.name == name)?;
        let lent: Vec<(String, String)> = extension
            .secrets
            .iter()
            .filter(|name| !name.starts_with("ORBITE_"))
            .filter_map(|name| Some((name.clone(), secrets?.get(name).ok()??)))
            .collect();
        let call = "globalThis.__settled = null;
             if (typeof check !== 'function') { __settled = { none: true }; }
             else Promise.resolve().then(() => check()).then(
               value => { __settled = { ok: value === undefined || value === null ? 'It works.'
                 : typeof value === 'string' ? value : JSON.stringify(value, null, 1) }; },
               error => { __settled = { error: String((error && error.message) || error) }; });"
            .to_owned();
        let runner = Arc::clone(self);
        let handle = tokio::runtime::Handle::current();
        let ran =
            tokio::task::spawn_blocking(move || runner.run(&extension, call, Some((handle, lent))))
                .await;
        match ran {
            Ok(Ok(settled)) => match serde_json::from_str::<Value>(&settled) {
                Ok(Value::Object(settled)) if settled.contains_key("none") => None,
                Ok(Value::Object(settled)) if settled.contains_key("ok") => {
                    Some(Ok(settled["ok"].as_str().unwrap_or_default().to_owned()))
                }
                Ok(Value::Object(settled)) => {
                    Some(Err(settled["error"].as_str().unwrap_or("?").to_owned()))
                }
                _ => Some(Err("it did not finish what it started".into())),
            },
            Ok(Err(Failure::Threw(problem) | Failure::Broke(problem))) => Some(Err(problem)),
            Err(_) => Some(Err("it crashed".into())),
        }
    }

    /// Turns off an Extension that does not work; the text tells the model.
    fn broke(&self, name: &str, problem: &str) -> String {
        self.turn_off(name, problem);
        // The model is told here and now: no need to tell it again.
        self.reports.lock().unwrap().pop();
        format!(
            "The Extension {name} does not work and was turned off: {problem}. Tell your Owner; they can \
             turn it back on with /extensions on {name}."
        )
    }

    fn turn_off(&self, name: &str, reason: &str) {
        let folder = self.folder.join(name);
        let _ = fs::create_dir_all(&folder);
        let _ = fs::write(folder.join("off"), reason);
        self.reports.lock().unwrap().push(format!(
            "The Extension {name} was turned off: {reason}. Your Owner has not been told: tell them in a \
             sentence. They can turn it back on with /extensions on {name}."
        ));
    }

    /// What the Agent has yet to be told about Extensions turned off.
    pub(crate) fn take_reports(&self) -> Vec<String> {
        std::mem::take(&mut self.reports.lock().unwrap())
    }

    /// Turns an Extension on or off for the Owner; false when there is none of that name.
    pub(crate) fn turn(&self, name: &str, on: bool) -> bool {
        if !self.all().iter().any(|extension| extension.name == name) {
            return false;
        }
        let folder = self.folder.join(name);
        if on {
            let _ = fs::remove_file(folder.join("off"));
        } else {
            let _ = fs::create_dir_all(&folder);
            let _ = fs::write(folder.join("off"), BY_THE_OWNER);
        }
        true
    }

    /// Removes an Extension and what it kept. Official ones cannot be removed.
    pub(crate) fn remove(&self, name: &str) -> Result<(), Unremovable> {
        match self.all().iter().find(|extension| extension.name == name) {
            None => Err(Unremovable::Unknown),
            Some(extension) if extension.official => Err(Unremovable::Official),
            Some(_) => fs::remove_dir_all(self.folder.join(name)).map_err(|_| Unremovable::Unknown),
        }
    }

    /// Runs `extension` in a sandbox of its own and returns what `call` leaves.
    fn run(
        &self,
        extension: &Extension,
        call: String,
        reaching: Option<(tokio::runtime::Handle, Vec<(String, String)>)>,
    ) -> Result<String, Failure> {
        // Saying what it adds takes an Extension no time at all.
        let limit = if reaching.is_some() {
            self.time_limit
        } else {
            self.time_limit.min(Duration::from_secs(2))
        };
        // One run at a time of an Extension that keeps things, in this
        // program and in any other of the installation: a second run would
        // start from what the first had not kept yet, and overwrite it. The
        // wait is not counted against the script.
        let _alone = match &reaching {
            Some(_) => {
                use std::os::fd::AsRawFd;
                let folder = self.folder.join(&extension.name);
                let lock = fs::create_dir_all(&folder)
                    .and_then(|()| fs::File::create(folder.join("data.lock")))
                    .map_err(|error| {
                        Failure::Threw(format!("it could not be run alone: {error}"))
                    })?;
                // SAFETY: a lock on a file opened just above, let go when it is closed.
                if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
                    let error = std::io::Error::last_os_error();
                    return Err(Failure::Threw(format!("it could not be run alone: {error}")));
                }
                Some(lock)
            }
            None => None,
        };
        let deadline = Instant::now() + limit;
        // Whatever went wrong, running out of time is what is reported.
        let failed = move |problem: String, breaks: bool| {
            if Instant::now() >= deadline {
                Failure::Broke(format!("it ran for more than {} seconds", limit.as_secs().max(1)))
            } else if breaks
                || problem.contains("out of memory")
                || problem.contains("stack overflow")
            {
                Failure::Broke(problem)
            } else {
                Failure::Threw(problem)
            }
        };
        let runtime = Runtime::new().map_err(|error| Failure::Broke(error.to_string()))?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        runtime.set_max_stack_size(512 * 1024);
        runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
        let context = Context::full(&runtime).map_err(|error| Failure::Broke(error.to_string()))?;

        let data = self.folder.join(&extension.name).join("data.json");
        // A script only asked what it adds is not an Extension yet: it is
        // given nothing that was kept, and keeps nothing.
        let describing = reaching.is_none();
        let kept: BTreeMap<String, Value> = if describing {
            BTreeMap::new()
        } else {
            fs::read_to_string(&data)
                .ok()
                .and_then(|kept| serde_json::from_str(&kept).ok())
                .unwrap_or_default()
        };
        // How much is kept under each key, so that the whole need not be
        // weighed again each time something is kept.
        let sizes: HashMap<String, usize> = kept
            .iter()
            .map(|(key, value)| (key.clone(), key.len() + value.to_string().len()))
            .collect();
        let weighed = Arc::new(Mutex::new(sizes));
        let store = Arc::new(Mutex::new((kept, false)));

        let loaded = context.with(|ctx| -> Result<(), String> {
            let kept = Arc::clone(&store);
            lend(
                &ctx,
                "__store_get",
                Function::new(ctx.clone(), move |key: String| {
                    kept.lock().unwrap().0.get(&key).map(Value::to_string).unwrap_or_default()
                }),
            )?;
            let kept = Arc::clone(&store);
            lend(
                &ctx,
                "__store_set",
                Function::new(ctx.clone(), {
                    let sizes = Arc::clone(&weighed);
                    move |key: String, value: String| -> String {
                        // Kept by Orbite, outside the engine's own limit: it has its own.
                        if value.len() > VALUE_LIMIT {
                            return format!(
                                "a value kept cannot be larger than {} kB",
                                VALUE_LIMIT / 1000
                            );
                        }
                        let Ok(parsed) = serde_json::from_str::<Value>(&value) else {
                            return "only what JSON can hold can be kept".into();
                        };
                        let mut kept = kept.lock().unwrap();
                        let mut sizes = sizes.lock().unwrap();
                        let held: usize = sizes
                            .iter()
                            .filter(|(other, _)| **other != key)
                            .map(|(_, size)| size)
                            .sum();
                        if held + key.len() + value.len() > STORE_LIMIT {
                            return format!(
                                "an Extension cannot keep more than {} MB in all",
                                STORE_LIMIT / 1_000_000
                            );
                        }
                        sizes.insert(key.clone(), key.len() + value.len());
                        kept.0.insert(key, parsed);
                        kept.1 = true;
                        String::new()
                    }
                }),
            )?;
            let kept = Arc::clone(&store);
            lend(
                &ctx,
                "__store_remove",
                Function::new(ctx.clone(), {
                    let sizes = Arc::clone(&weighed);
                    move |key: String| {
                        let mut kept = kept.lock().unwrap();
                        sizes.lock().unwrap().remove(&key);
                        kept.1 |= kept.0.remove(&key).is_some();
                    }
                }),
            )?;
            let kept = Arc::clone(&store);
            lend(
                &ctx,
                "__store_keys",
                Function::new(ctx.clone(), move || {
                    json!(kept.lock().unwrap().0.keys().collect::<Vec<_>>()).to_string()
                }),
            )?;
            let connecting = reaching.clone();
            let (http, hosts) = (self.http.clone(), extension.hosts.clone());
            lend(
                &ctx,
                "__http",
                Function::new(ctx.clone(), move |request: String| {
                    let answer = match &reaching {
                        None => {
                            Err("nothing can be reached while an Extension describes itself"
                                .to_owned())
                        }
                        Some((handle, lent)) => {
                            // Whatever goes wrong, the script is told a
                            // secret's name, never its value.
                            handle
                                .block_on(reach(&http, &hosts, lent, &request, deadline))
                                .map_err(|problem| named(&problem, lent))
                        }
                    };
                    answer.unwrap_or_else(|problem| json!({ "error": problem })).to_string()
                }),
            )?;
            let (ports, wires) = (extension.hosts.clone(), Arc::new(Wires::default()));
            lend(
                &ctx,
                "__net",
                Function::new(ctx.clone(), move |request: String| {
                    let answer = match &connecting {
                        None => {
                            Err("nothing can be reached while an Extension describes itself"
                                .to_owned())
                        }
                        Some((handle, lent)) => handle
                            .block_on(wire(&wires, &ports, lent, &request, deadline))
                            .map_err(|problem| named(&problem, lent)),
                    };
                    answer.unwrap_or_else(|problem| json!({ "error": problem })).to_string()
                }),
            )?;
            ctx.eval::<(), _>(LENT).map_err(|_| caught(&ctx))?;
            ctx.eval::<(), _>(extension.script.as_str()).map_err(|_| caught(&ctx))
        });
        loaded.map_err(|problem| failed(problem, true))?;

        let result = if call.starts_with("JSON.stringify") {
            context.with(|ctx| ctx.eval::<String, _>(call.as_str()).map_err(|_| caught(&ctx)))
        } else {
            context.with(|ctx| ctx.eval::<(), _>(call.as_str()).map_err(|_| caught(&ctx))).and_then(
                |()| {
                    // What the script awaits is settled before its answer is read.
                    while runtime.is_job_pending() {
                        if runtime.execute_pending_job().is_err() {
                            break;
                        }
                    }
                    context.with(|ctx| {
                        ctx.eval::<String, _>("JSON.stringify(__settled)").map_err(|_| caught(&ctx))
                    })
                },
            )
        };
        let result = result.map_err(|problem| failed(problem, false))?;
        if Instant::now() >= deadline {
            return Err(failed(String::new(), false));
        }

        let store = store.lock().unwrap();
        if store.1 && !describing {
            let folder = self.folder.join(&extension.name);
            let being_written = folder.join("data.writing");
            let written = fs::create_dir_all(&folder)
                .and_then(|()| {
                    fs::write(
                        &being_written,
                        serde_json::to_string_pretty(&store.0).unwrap_or_default(),
                    )
                })
                .and_then(|()| fs::rename(&being_written, &data));
            if let Err(error) = written {
                return Err(Failure::Threw(format!("what it keeps could not be written: {error}")));
            }
        }
        Ok(result)
    }
}

/// A connection an Extension holds to a port it was granted.
enum Wire {
    Plain(tokio::net::TcpStream),
    Secure(Box<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>),
}

/// A connection that is held.
struct Held {
    /// The name it was opened to.
    host: String,
    wire: Wire,
    /// What was read from it and not yet given to the script.
    read: Vec<u8>,
    /// How long it took to reach: what a network cuts in two comes back
    /// together within a few of these.
    reached_in: Duration,
}

/// The connections of one run, by the number the script knows each by.
#[derive(Default)]
struct Wires {
    held: Mutex<HashMap<u32, Held>>,
    opened: Mutex<u32>,
}

/// What a connection is checked against: the certificates this machine trusts.
fn trusting() -> Result<tokio_rustls::TlsConnector, String> {
    use rustls_platform_verifier::ConfigVerifierExt;
    let config = rustls::ClientConfig::with_platform_verifier()
        .map_err(|error| format!("this machine's certificates could not be read: {error}"))?;
    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

/// Carries out what a script asks of a connection: opening one to a port
/// it was granted, writing to it, reading from it, encrypting it, closing it.
async fn wire(
    wires: &Wires,
    hosts: &[String],
    lent: &[(String, String)],
    request: &str,
    deadline: Instant,
) -> Result<Value, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[derive(Deserialize)]
    struct Asked {
        op: String,
        #[serde(default)]
        id: u32,
        #[serde(default)]
        host: String,
        #[serde(default)]
        port: u16,
        #[serde(default)]
        tls: bool,
        data: Option<String>,
        until: Option<String>,
        timeout: Option<f64>,
    }
    let asked: Asked = serde_json::from_str(request).map_err(|error| error.to_string())?;
    let left = || deadline.saturating_duration_since(Instant::now());
    let encrypted = |name: String, plain: tokio::net::TcpStream| async move {
        let server = rustls::pki_types::ServerName::try_from(name.clone())
            .map_err(|_| format!("{name} is not a name a certificate can be checked against"))?;
        let secure = tokio::time::timeout(left(), trusting()?.connect(server, plain))
            .await
            .map_err(|_| format!("{name} did not answer in time"))?
            .map_err(|error| format!("{name} could not be reached encrypted: {error}"))?;
        Ok::<_, String>(Wire::Secure(Box::new(secure)))
    };
    if asked.op == "connect" {
        let host = asked.host.trim().to_lowercase();
        let wanted = format!("{host}:{}", asked.port);
        if !hosts.iter().any(|granted| granted.trim().to_lowercase() == wanted) {
            return Err(format!(
                "this Extension was not granted {wanted}: it can only reach {}",
                listed(hosts)
            ));
        }
        if wires.held.lock().unwrap().len() >= CONNECTIONS_AT_ONCE {
            return Err(format!(
                "an Extension holds at most {CONNECTIONS_AT_ONCE} connections at once: close one"
            ));
        }
        let asked_at = Instant::now();
        let plain = tokio::time::timeout(left(), tokio::net::TcpStream::connect(&wanted))
            .await
            .map_err(|_| format!("{wanted} did not answer in time"))?
            .map_err(|error| format!("{wanted} could not be reached: {error}"))?;
        let reached_in = asked_at.elapsed();
        // Sent as it is written, not gathered: less of it is cut in two.
        let _ = plain.set_nodelay(true);
        let wire =
            if asked.tls { encrypted(host.clone(), plain).await? } else { Wire::Plain(plain) };
        let id = {
            let mut opened = wires.opened.lock().unwrap();
            *opened += 1;
            *opened
        };
        if id > CONNECTIONS_IN_ALL {
            return Err(format!(
                "an Extension opens at most {CONNECTIONS_IN_ALL} connections in one run"
            ));
        }
        let held = Held { host, wire, read: Vec::new(), reached_in };
        wires.held.lock().unwrap().insert(id, held);
        return Ok(json!({ "id": id }));
    }
    // Taken out while it is used, and put back unless it was closed.
    let taken = wires.held.lock().unwrap().remove(&asked.id);
    let Some(Held { host, mut wire, mut read, reached_in }) = taken else {
        return Err("this connection is closed".into());
    };
    let answer = match asked.op.as_str() {
        "close" => return Ok(json!({})),
        "starttls" => match wire {
            Wire::Secure(_) => Err(format!("the connection to {host} is already encrypted")),
            Wire::Plain(plain) => {
                wire = encrypted(host.clone(), plain).await?;
                Ok(json!({}))
            }
        },
        "write" => {
            // A secret is filled in on the way out, as on the web.
            let data = filled(asked.data.as_deref().unwrap_or_default(), lent)?;
            let wrote = async {
                match &mut wire {
                    Wire::Plain(plain) => plain.write_all(data.as_bytes()).await,
                    Wire::Secure(secure) => {
                        secure.write_all(data.as_bytes()).await?;
                        secure.flush().await
                    }
                }
            };
            match tokio::time::timeout(left(), wrote).await {
                Err(_) => Err(format!("{host} did not take it in time")),
                Ok(Err(error)) => Err(format!("{host} could not be written to: {error}")),
                Ok(Ok(())) => Ok(json!({})),
            }
        }
        "read" => {
            // In seconds; a wait written in thousandths, as scripts often
            // do, is taken as such. Never longer than the run has left.
            let wait = match asked.timeout.filter(|wait| wait.is_finite() && *wait > 0.0) {
                None => READ_WAIT,
                Some(wait) if wait > 120.0 => Duration::from_secs_f64((wait / 1000.0).min(60.0)),
                Some(wait) => Duration::from_secs_f64(wait.min(60.0)),
            }
            .min(left());
            let until: Option<String> = asked
                .until
                .filter(|mark| !mark.is_empty())
                .map(|mark| mark.chars().take(MARK_LIMIT).collect());
            let stop = Instant::now() + wait;
            let mut part = vec![0u8; 16 * 1024];
            let mut closed = false;
            // A secret a service says back may come in pieces, and how much
            // has come at a given instant says something of how long it is.
            // So an Extension that was lent a secret is handed what a
            // service says only once it is all there: nothing more has come
            // for a moment, or the connection closed. By then a secret is
            // whole, and found. The moment is a few times what the service
            // takes to reach, since that is what a network cuts apart by.
            let careful = lent.iter().any(|(_, value)| !value.is_empty());
            let quiet = (reached_in * 3 + QUIET).min(QUIET_AT_MOST);
            let mut settled = false;
            // The mark is looked for in what the script would be given,
            // where a secret is already a name.
            let (text, taken) = loop {
                let whole = !careful || settled || closed;
                // More than is ever held at once, and still coming: what is
                // there is handed over short of its end, where a secret may
                // not all have come. The same is handed whatever a secret
                // is, and however long.
                let full = !whole && read.len() >= BODY_LIMIT * 4;
                let (text, taken) = if whole {
                    handed_over(&read, lent, 0)
                } else if full {
                    handed_over(&read, lent, longest_dress(lent).saturating_sub(1))
                } else {
                    (String::new(), 0)
                };
                if full {
                    break (text, taken);
                }
                let found = whole
                    && match &until {
                        Some(mark) => text.contains(mark.as_str()),
                        None => taken > 0,
                    };
                if found || closed || (whole && read.len() >= BODY_LIMIT * 4) {
                    break (text, taken);
                }
                let waiting_for_quiet = !whole && !read.is_empty();
                let patience = if waiting_for_quiet {
                    // Not for ever: a service that never stops talking does
                    // not hold a read much past the wait it was given.
                    let last = (stop + quiet).saturating_duration_since(Instant::now());
                    quiet.min(left()).min(last)
                } else {
                    stop.saturating_duration_since(Instant::now())
                };
                if patience.is_zero() {
                    break (text, taken);
                }
                let reading = async {
                    match &mut wire {
                        Wire::Plain(plain) => plain.read(&mut part).await,
                        Wire::Secure(secure) => secure.read(&mut part).await,
                    }
                };
                match tokio::time::timeout(patience, reading).await {
                    // Nothing more for a moment: what has come is all of it.
                    Err(_) if waiting_for_quiet => settled = true,
                    Err(_) => break (text, taken),
                    Ok(Ok(0)) | Ok(Err(_)) => closed = true,
                    Ok(Ok(count)) => {
                        read.extend_from_slice(&part[..count]);
                        settled = false;
                    }
                }
            };
            read.drain(..taken);
            if closed && text.is_empty() && read.is_empty() {
                return Err(format!("{host} closed the connection"));
            }
            Ok(json!({ "data": text }))
        }
        other => Err(format!("a connection cannot {other}")),
    };
    let held = Held { host, wire, read, reached_in };
    wires.held.lock().unwrap().insert(asked.id, held);
    answer
}

/// Lends `function` to the scripts of `ctx` under `name`.
fn lend<'js>(
    ctx: &rquickjs::Ctx<'js>,
    name: &str,
    function: rquickjs::Result<Function<'js>>,
) -> Result<(), String> {
    ctx.globals().set(name, function.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// What the names of an Extension's Actions start with: its own name, in
/// the letters every model accepts in the name of an Action.
fn prefix(name: &str) -> String {
    name.replace('-', "_")
}

/// Why an Extension was not removed.
pub(crate) enum Unremovable {
    Unknown,
    Official,
}

/// The error a script just threw, in words.
fn caught(ctx: &rquickjs::Ctx) -> String {
    let thrown = ctx.catch();
    if let Some(exception) = thrown.as_exception() {
        return exception.message().unwrap_or_else(|| "an error".into());
    }
    thrown
        .as_string()
        .and_then(|text| text.to_string().ok())
        .unwrap_or_else(|| "it was stopped".into())
}

/// Carries a script's request to a host it was granted, and brings back the answer.
async fn reach(
    http: &reqwest::Client,
    hosts: &[String],
    lent: &[(String, String)],
    request: &str,
    deadline: Instant,
) -> Result<Value, String> {
    #[derive(Deserialize)]
    struct Asked {
        method: String,
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        body: Option<String>,
    }
    let mut asked: Asked = serde_json::from_str(request).map_err(|error| error.to_string())?;
    asked.url = filled(&asked.url, lent)?;
    for value in asked.headers.values_mut() {
        *value = filled(value, lent)?;
    }
    if let Some(body) = &mut asked.body {
        *body = filled(body, lent)?;
    }
    let mut url =
        url::Url::parse(&asked.url).map_err(|_| format!("{} is not an address", asked.url))?;
    let mut method =
        reqwest::Method::from_bytes(asked.method.as_bytes()).map_err(|error| error.to_string())?;
    let mut body = asked.body;
    let granted = |url: &url::Url| {
        let host = url.host_str().unwrap_or_default().to_lowercase();
        let known = matches!(url.scheme(), "http" | "https")
            && hosts.iter().any(|granted| granted.to_lowercase() == host);
        if known { Ok(host) } else { Err(host) }
    };
    let mut host = granted(&url).map_err(|host| {
        format!("this Extension was not granted {host}: it can only reach {}", listed(hosts))
    })?;
    // A redirection is followed by hand, and only to a host that was granted.
    for _ in 0..REDIRECTIONS_FOLLOWED {
        let mut sending = http
            .request(method.clone(), url.clone())
            .timeout(deadline.saturating_duration_since(Instant::now()));
        for (name, value) in &asked.headers {
            sending = sending.header(name, value);
        }
        if let Some(body) = &body {
            sending = sending.body(body.clone());
        }
        let answer = sending.send().await.map_err(|error| {
            // Without the address, which may carry a secret.
            format!("{host} could not be reached: {}", error.without_url())
        })?;
        let status = answer.status();
        let moved = answer
            .headers()
            .get("location")
            .and_then(|to| to.to_str().ok())
            .and_then(|to| url.join(to).ok());
        if let (true, Some(to)) = (status.is_redirection(), moved) {
            host = granted(&to).map_err(|other| {
format!(
                    "{host} redirects to {to}, and this Extension was not granted {other}: it can only reach {}",
                    listed(hosts)
                )
            })?;
            // As browsers do: what was sent is sent again only when asked to.
            if !matches!(status.as_u16(), 307 | 308) {
                method = reqwest::Method::GET;
                body = None;
            }
            url = to;
            continue;
        }
        // Read as it comes and no further than the limit: a service cannot
        // make Orbite hold more than a script would be given.
        let mut answer = answer;
        let mut read: Vec<u8> = Vec::new();
        while read.len() < BODY_LIMIT * 4 {
            match answer.chunk().await {
                Ok(Some(part)) => read.extend_from_slice(&part),
                Ok(None) => break,
                Err(error) => {
                    return Err(format!("{host} did not answer whole: {}", error.without_url()));
                }
            }
        }
        // A service that sends a secret back does not hand it to the script:
        // named before the text is cut, so that no part of one is left. A
        // body longer than is read stops short of where it was cut off,
        // where a secret may not all have been read: found in the whole of
        // what was read first, one that started before there is whole.
        let text: String = if read.len() >= BODY_LIMIT * 4 {
            handed_over(&read, lent, longest_dress(lent).saturating_sub(1)).0
        } else {
            named(&String::from_utf8_lossy(&read), lent).chars().take(BODY_LIMIT).collect()
        };
        return Ok(json!({ "status": status.as_u16(), "body": text }));
    }
    Err(format!("{host} redirects again and again"))
}

/// `text` with each {{NAME}} replaced by the secret of that name, when the
/// Extension was granted it.
pub(crate) fn filled(text: &str, lent: &[(String, String)]) -> Result<String, String> {
    let mut filled = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("{{") {
        filled.push_str(before);
        let Some((name, after)) = after.split_once("}}") else {
            filled.push_str("{{");
            rest = after;
            continue;
        };
        // A password is often asked for in base64 (SMTP's AUTH LOGIN): the
        // script names it so, and still never holds it.
        let (name, in_base64) = match name.strip_prefix("base64:") {
            Some(name) => (name, true),
            None => (name, false),
        };
        match lent.iter().find(|(granted, _)| granted == name) {
            Some((_, value)) if in_base64 => {
                use base64::Engine;
                filled.push_str(&base64::engine::general_purpose::STANDARD.encode(value));
            }
            Some((_, value)) => filled.push_str(value),
            None => {
                return Err(format!(
                    "this Extension was not granted the secret {name}, or your Owner has not entered it yet \
                     (orbite secret set {name})"
                ));
            }
        }
        rest = after;
    }
    Ok(filled + rest)
}

/// The dresses a secret may come back in: as it is, in other capitals, as
/// an address carries it, and in base64 of either alphabet, with or
/// without its padding.
fn dresses(value: &str) -> Vec<String> {
    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let mut dresses = vec![value.to_owned(), value.to_lowercase(), value.to_uppercase()];
    let encoded: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
    if encoded != *value {
        dresses.push(encoded);
    }
    dresses.extend([
        STANDARD.encode(value),
        STANDARD_NO_PAD.encode(value),
        URL_SAFE.encode(value),
        URL_SAFE_NO_PAD.encode(value),
    ]);
    dresses.retain(|dress| !dress.is_empty());
    dresses
}

/// `text` with the value of each secret lent replaced by its name, in the
/// forms an address or an error may give it.
fn named(text: &str, lent: &[(String, String)]) -> String {
    let mut named = text.to_owned();
    for (name, value) in lent {
        let marked = format!("[{name}]");
        for dress in dresses(value) {
            named = named.replace(&dress, &marked);
        }
    }
    named
}

/// How long the longest form is that a secret lent may come back in.
fn longest_dress(lent: &[(String, String)]) -> usize {
    lent.iter().flat_map(|(_, value)| dresses(value)).map(|dress| dress.len()).max().unwrap_or(0)
}

/// What of `read` a script is handed, and how many of its bytes that
/// takes. Every secret in the whole of it, in any dress, is found first
/// and given as its name, however often it is there. It is then measured
/// as the script will see it, a secret counting for its name, and cut at
/// what a script may be given at once: never inside a secret, nor inside
/// a letter, and nowhere that tells how long a secret is. When more is
/// still to come, it stops `short_by` bytes before the end of what has:
/// a secret that started before there is whole, and one that started
/// after is not handed over at all.
fn handed_over(read: &[u8], lent: &[(String, String)], short_by: usize) -> (String, usize) {
    // Whose secret each byte is part of, if any.
    let mut whose: Vec<Option<usize>> = vec![None; read.len()];
    for (index, (_, value)) in lent.iter().enumerate() {
        for dress in dresses(value) {
            let dress = dress.as_bytes();
            if dress.len() > read.len() {
                continue;
            }
            for start in 0..=read.len() - dress.len() {
                if read[start] == dress[0] && read[start..start + dress.len()] == *dress {
                    for part in &mut whose[start..start + dress.len()] {
                        part.get_or_insert(index);
                    }
                }
            }
        }
    }
    // The runs of it: plain, or a secret, with what each weighs once shown.
    let mut runs: Vec<(usize, usize, Option<usize>, usize)> = Vec::new();
    let mut at = 0;
    while at < read.len() {
        let mut end = at;
        while end < read.len() && whose[end].is_some() == whose[at].is_some() {
            end += 1;
        }
        let weight = match whose[at] {
            Some(index) => lent[index].0.len() + 2,
            None => end - at,
        };
        runs.push((at, end, whose[at], weight));
        at = end;
    }
    let mut room = BODY_LIMIT;
    let last = read.len() - short_by.min(read.len());
    let (mut text, mut taken) = (String::new(), 0);
    for (start, end, secret, weight) in runs {
        if start >= last {
            break;
        }
        match secret {
            // A secret is given whole, as its name, or not yet.
            Some(index) if weight <= room => {
                text.push_str(&format!("[{}]", lent[index].0));
                room -= weight;
                taken = end;
            }
            Some(_) => break,
            None => {
                let mut upto = (start + weight.min(room)).min(last);
                // Not inside a letter.
                for _ in 0..3 {
                    if upto > start && upto < read.len() && (read[upto] & 0xC0) == 0x80 {
                        upto -= 1;
                    }
                }
                text.push_str(&String::from_utf8_lossy(&read[start..upto]));
                taken = upto;
                if upto < end {
                    break;
                }
                room -= weight.min(room);
            }
        }
    }
    (text, taken)
}

fn listed(hosts: &[String]) -> String {
    if hosts.is_empty() { "nothing".into() } else { hosts.join(", ") }
}

/// The Actions an Extension says it adds, or what is wrong with what it says.
fn understand(said: &str) -> Result<Vec<Described>, String> {
    let said: Value =
        serde_json::from_str(said).map_err(|_| "describe() must return a list".to_owned())?;
    let said = said.as_array().ok_or("describe() must return a list")?;
    let mut described = Vec::new();
    for action in said {
        let name = action["name"].as_str().unwrap_or_default();
        if name.is_empty()
            || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(format!(
                "an action's name must be lowercase letters, digits and _, not {:?}",
                action["name"]
            ));
        }
        let description = action["description"].as_str().unwrap_or_default();
        if description.is_empty() {
            return Err(format!("the action {name} has no description"));
        }
        // A parameter is its description, or its whole schema; a name
        // ending in ? is one that may be left out.
        let (mut properties, mut required) = (serde_json::Map::new(), Vec::new());
        for (parameter, about) in action["parameters"].as_object().into_iter().flatten() {
            let named = parameter.trim_end_matches('?');
            if named == parameter {
                required.push(named.to_owned());
            }
            let schema = match about {
                Value::String(description) => {
                    json!({ "type": "string", "description": description })
                }
                other => other.clone(),
            };
            properties.insert(named.to_owned(), schema);
        }
        described.push(Described {
            name: name.to_owned(),
            description: description.to_owned(),
            input: json!({ "type": "object", "properties": properties, "required": required }),
            asks: action["asks"].as_bool().unwrap_or(false),
            critical: action["critical"].as_bool().unwrap_or(false),
        });
    }
    Ok(described)
}

/// The first sentence of `text`, without its full stop.
fn one_sentence(text: &str) -> &str {
    text.split_once(". ").map_or(text, |(first, _)| first).trim_end_matches('.')
}
