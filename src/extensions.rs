//! Extensions: scripts that add Actions to the Agent, run in a sandbox where
//! they reach nothing but what Orbite lends them: a store of their own and
//! the hosts they were granted.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rquickjs::{Context, Function, Runtime};
use serde::Deserialize;
use serde_json::{Value, json};

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

/// The reason kept for an Extension its Owner turned off.
pub(crate) const BY_THE_OWNER: &str = "by the Owner";

/// What Orbite lends, as an Extension sees it.
const LENT: &str = r#"
const store = Object.freeze({
  get: key => { const kept = __store_get(String(key)); return kept === "" ? null : JSON.parse(kept); },
  set: (key, value) => {
    if (value === undefined || value === null) __store_remove(String(key));
    else __store_set(String(key), JSON.stringify(value));
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
}

/// One Extension of this installation.
pub(crate) struct Extension {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) description_fr: Option<String>,
    hosts: Vec<String>,
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
            .filter(|name| slug(name).is_ok_and(|safe| &safe == name))
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
                specs.push(ActionSpec {
                    name: format!("{}_{}", extension.name, action.name),
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
        let said = self
            .run(extension, "JSON.stringify(describe())".into(), None)
            .and_then(|said| understand(&said).map_err(Failure::Broke));
        match said {
            Ok(described) => {
                self.described.lock().unwrap().insert(key, described.clone());
                Some(described)
            }
            Err(Failure::Threw(problem) | Failure::Broke(problem)) => {
                self.turn_off(&extension.name, &format!("it could not be loaded: {problem}"));
                None
            }
        }
    }

    /// The Extension and Action a request is for, when it is for one.
    fn find(&self, request: &ActionRequest) -> Option<(Extension, Described)> {
        self.all().into_iter().filter(|extension| extension.off.is_none()).find_map(|extension| {
            let action = request.name.strip_prefix(&extension.name)?.strip_prefix('_')?;
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
    pub(crate) async fn carry_out(self: &Arc<Self>, request: &ActionRequest) -> Option<String> {
        let (extension, action) = self.find(request)?;
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
        let runner = Arc::clone(self);
        let handle = tokio::runtime::Handle::current();
        let ran =
            tokio::task::spawn_blocking(move || runner.run(&extension, call, Some(handle))).await;
        Some(match ran {
            Ok(Ok(settled)) => match serde_json::from_str::<Value>(&settled) {
                Ok(Value::Object(settled)) if settled.contains_key("ok") => {
                    settled["ok"].as_str().unwrap_or_default().to_owned()
                }
                Ok(Value::Object(settled)) => match settled["error"].as_str().unwrap_or("?") {
                    problem if problem.contains("out of memory") => {
                        self.broke(&name, "it took more memory than an Extension may")
                    }
                    problem => format!("The Extension {name} could not do it: {problem}"),
                },
                _ => format!("The Extension {name} did not finish what it started."),
            },
            Ok(Err(Failure::Threw(problem))) => {
                format!("The Extension {name} could not do it: {problem}")
            }
            Ok(Err(Failure::Broke(problem))) => self.broke(&name, &problem),
            Err(_) => self.broke(&name, "it crashed"),
        })
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
        reaching: Option<tokio::runtime::Handle>,
    ) -> Result<String, Failure> {
        // Saying what it adds takes an Extension no time at all.
        let limit = if reaching.is_some() {
            self.time_limit
        } else {
            self.time_limit.min(Duration::from_secs(2))
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
        let kept: BTreeMap<String, Value> = fs::read_to_string(&data)
            .ok()
            .and_then(|kept| serde_json::from_str(&kept).ok())
            .unwrap_or_default();
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
                Function::new(ctx.clone(), move |key: String, value: String| {
                    if let Ok(value) = serde_json::from_str(&value) {
                        let mut kept = kept.lock().unwrap();
                        kept.0.insert(key, value);
                        kept.1 = true;
                    }
                }),
            )?;
            let kept = Arc::clone(&store);
            lend(
                &ctx,
                "__store_remove",
                Function::new(ctx.clone(), move |key: String| {
                    let mut kept = kept.lock().unwrap();
                    kept.1 |= kept.0.remove(&key).is_some();
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
                        Some(handle) => handle.block_on(reach(&http, &hosts, &request, deadline)),
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
        if store.1 {
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

/// Lends `function` to the scripts of `ctx` under `name`.
fn lend<'js>(
    ctx: &rquickjs::Ctx<'js>,
    name: &str,
    function: rquickjs::Result<Function<'js>>,
) -> Result<(), String> {
    ctx.globals().set(name, function.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
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
    let asked: Asked = serde_json::from_str(request).map_err(|error| error.to_string())?;
    let url =
        url::Url::parse(&asked.url).map_err(|_| format!("{} is not an address", asked.url))?;
    let host = url.host_str().unwrap_or_default().to_lowercase();
    if !matches!(url.scheme(), "http" | "https")
        || !hosts.iter().any(|granted| granted.to_lowercase() == host)
    {
        return Err(format!(
            "this Extension was not granted {host}: it can only reach {}",
            listed(hosts)
        ));
    }
    let method =
        reqwest::Method::from_bytes(asked.method.as_bytes()).map_err(|error| error.to_string())?;
    let mut sending =
        http.request(method, url).timeout(deadline.saturating_duration_since(Instant::now()));
    for (name, value) in &asked.headers {
        sending = sending.header(name, value);
    }
    if let Some(body) = asked.body {
        sending = sending.body(body);
    }
    let answer =
        sending.send().await.map_err(|error| format!("{host} could not be reached: {error}"))?;
    let status = answer.status().as_u16();
    let body =
        answer.text().await.map_err(|error| format!("{host} did not answer whole: {error}"))?;
    let body: String = body.chars().take(BODY_LIMIT).collect();
    Ok(json!({ "status": status, "body": body }))
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
