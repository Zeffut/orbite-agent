//! The server that holds several Agents (ADR 0022): it lets the Owner's
//! devices in, lists, creates and removes Agents, each in a container of
//! its own, and carries what a device says to the Agent it is for.

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path as Named, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::interface::{came_encrypted, device_in, from_the_page, kept_in, let_in, refused};

/// How much an Agent sends at once to be thought about, pictures included.
const THOUGHT_AT_MOST: usize = 32 * 1024 * 1024;

/// How many things one Agent has the server thinking about at once: itself,
/// and those it sent to work beside it.
const THOUGHTS_AT_ONCE: usize = 4;

/// How many Agents one server holds at the most, unless told otherwise.
const AGENTS_AT_MOST: usize = 8;

/// One Agent of the server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Held {
    /// What it is known by in addresses and to the engine: a few lowercase
    /// letters and digits, its own for good.
    pub id: String,
    /// What the Owner called it when it was made.
    pub name: String,
    /// What the server shows the Agent to be known by it.
    pub(crate) token: String,
    /// How many tokens of the Owner's Providers it may use in a day, when
    /// its Owner set a limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) daily_tokens: Option<u64>,
}

impl Held {
    /// One more Agent, called `name`, known by nothing another is.
    pub fn new(name: &str, others: &[Held]) -> io::Result<Self> {
        let mut id = random(6)?;
        while others.iter().any(|other| other.id == id) {
            id = random(6)?;
        }
        Ok(Self { id, name: name.to_owned(), token: random(32)?, daily_tokens: None })
    }
}

/// The Agents of the server kept in `home`: none when it is not one.
pub fn held_in(home: &Path) -> Vec<Held> {
    let held: Vec<Held> = std::fs::read_to_string(home.join("agents.json"))
        .ok()
        .and_then(|kept| serde_json::from_str(&kept).ok())
        .unwrap_or_default();
    // What an Agent is known by names its container, its volume and its
    // files: a few letters and digits, or it is none of this server's.
    held.into_iter().filter(|agent| is_an_id(&agent.id) && agent.token.len() >= 32).collect()
}

/// True for what a server calls an Agent by: a few lowercase letters and digits.
fn is_an_id(id: &str) -> bool {
    (1..=16).contains(&id.len())
        && id.bytes().all(|letter| letter.is_ascii_lowercase() || letter.is_ascii_digit())
}

/// What runs the Agents' containers. Docker, on a Host; something that
/// pretends, in tests.
pub trait Engine: Send + Sync + 'static {
    /// Makes and starts the container of the Agent `id`, giving it what
    /// the server has to tell it: what to trust the server by, what it is
    /// first called, where the server is.
    fn start(&self, id: &str, given: &[(String, String)]) -> Result<(), String>;
    /// The address of this machine as an Agent sees it from its container,
    /// when Agents can reach the server at all.
    fn seen_from_agents(&self) -> Option<std::net::IpAddr> {
        None
    }
    /// Where the Agent `id` is reached, once it runs: `http://address:port`.
    fn reached_at(&self, id: &str) -> Result<String, String>;
    /// Removes the container of the Agent `id` and everything it kept.
    fn remove(&self, id: &str) -> Result<(), String>;
    /// Clears what earlier versions left that nothing uses any more.
    fn tidy(&self) {}
}

/// Docker, asked through its own program.
pub struct Docker {
    /// The image an Agent runs from.
    pub image: String,
    /// True when that image is this machine's to build, from this very
    /// program: built the first time it is needed, once per version.
    pub built_here: bool,
}

/// How an Agent's image is made.
const DOCKERFILE: &str = include_str!("../container/Dockerfile");

/// Held while an image is built: one at a time.
static BUILDING: Mutex<()> = Mutex::new(());

impl Docker {
    /// Docker running Agents from the image of this version of Orbite,
    /// built on this machine from this program.
    pub fn of_this_version() -> Self {
        Self { image: format!("orbite-agent:{}", env!("CARGO_PKG_VERSION")), built_here: true }
    }

    /// True when Docker answers to whoever runs this.
    pub fn answers(&self) -> Result<(), String> {
        self.ask(&["version", "-f", "{{.Server.Version}}"]).map(|_| ())
    }

    /// Builds the image when it is this machine's to build and is not
    /// there yet. It takes a few minutes, once per version.
    pub fn built(&self) -> Result<(), String> {
        if !self.built_here {
            return Ok(());
        }
        let _one_at_a_time = BUILDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.ask(&["image", "inspect", "-f", "{{.Id}}", &self.image]).is_ok() {
            return Ok(());
        }
        let told = |error: io::Error| format!("the agent's image could not be prepared: {error}");
        let folder = tempfile::tempdir().map_err(told)?;
        std::fs::write(folder.path().join("Dockerfile"), DOCKERFILE).map_err(told)?;
        let program = std::env::current_exe().map_err(told)?;
        std::fs::copy(program, folder.path().join("orbite")).map_err(told)?;
        let folder = folder.path().to_string_lossy().into_owned();
        self.ask(&["build", "-q", "-t", &self.image, &folder]).map(|_| ())
    }

    /// Removes every image built here for Agents: for when Orbite goes.
    pub fn forget_images(&self) {
        let Ok(kept) =
            self.ask(&["image", "ls", "orbite-agent", "--format", "{{.Repository}}:{{.Tag}}"])
        else {
            return;
        };
        for image in kept.lines().filter(|image| image.starts_with("orbite-agent:")) {
            let _ = self.ask(&["image", "rm", image]);
        }
    }

    /// Removes the images of other versions that no container runs from.
    pub fn forget_other_versions(&self) {
        if !self.built_here {
            return;
        }
        let Ok(kept) =
            self.ask(&["image", "ls", "orbite-agent", "--format", "{{.Repository}}:{{.Tag}}"])
        else {
            return;
        };
        for other in kept.lines().filter(|other| *other != self.image) {
            // Refused while a container still runs from it: left alone then.
            let _ = self.ask(&["image", "rm", other]);
        }
    }

    /// Removes the container of the Agent `id`, and with `all` everything
    /// it kept as well.
    pub fn stop(&self, id: &str, all: bool) -> Result<(), String> {
        if all {
            return Engine::remove(self, id);
        }
        match self.ask(&["rm", "-f", &format!("orbite-agent-{id}")]) {
            Err(why) if !why.to_lowercase().contains("no such") => Err(why),
            _ => Ok(()),
        }
    }

    /// Writes everything the Agent `id` keeps to a file of its own in
    /// `folder`, as an installation's export is written: whole even while
    /// the Agent works. The file is named after the Agent.
    pub fn export_of(&self, id: &str, folder: &Path) -> Result<PathBuf, String> {
        self.built()?;
        let (kept, out) =
            (format!("orbite-agent-{id}:/var/lib/orbite"), format!("{}:/out", folder.display()));
        let written = format!("/out/{id}.orbite");
        self.ask(&[
            "run",
            "--rm",
            "--user",
            "0",
            "--network",
            "none",
            "--cap-drop=ALL",
            "--cap-add=CHOWN",
            "--cap-add=DAC_OVERRIDE",
            "--cap-add=FOWNER",
            "--security-opt=no-new-privileges",
            "-e",
            "ORBITE_SERVER_TOKEN=held-by-a-server-and-being-moved-now",
            "-v",
            &kept,
            "-v",
            &out,
            &self.image,
            "orbite",
            "export",
            &written,
        ])?;
        Ok(folder.join(format!("{id}.orbite")))
    }

    /// Puts what `file` holds, an Agent's own export, into the volume of
    /// the Agent `id`, which must not hold an Agent yet.
    pub fn restore_of(&self, id: &str, file: &Path) -> Result<(), String> {
        self.built()?;
        let named = format!("orbite-agent-{id}");
        self.ask(&["volume", "create", &named])?;
        let (kept, from) =
            (format!("{named}:/var/lib/orbite"), format!("{}:/in.orbite:ro", file.display()));
        self.ask(&[
            "run",
            "--rm",
            "--user",
            "0",
            "--network",
            "none",
            "--cap-drop=ALL",
            "--cap-add=CHOWN",
            "--cap-add=DAC_OVERRIDE",
            "--cap-add=FOWNER",
            "--security-opt=no-new-privileges",
            "-e",
            "ORBITE_SERVER_TOKEN=held-by-a-server-and-being-moved-now",
            "-v",
            &kept,
            "-v",
            &from,
            &self.image,
            "sh",
            "-c",
            // Put back beside the volume first, then copied into it: never
            // over an Agent that is there already.
            "[ ! -e /var/lib/orbite/config.json ] && [ ! -e /var/lib/orbite/orbite.db ] \
             && ORBITE_HOME=/tmp/agent orbite restore /in.orbite \
             && cp -a /tmp/agent/. /var/lib/orbite/ && chown -R orbite:orbite /var/lib/orbite",
        ])
        .map(|_| ())
    }

    /// True when the Agent `id` reaches the server from its container: the
    /// server answers it there, whatever it answers.
    pub fn reaches_the_server(&self, id: &str) -> bool {
        let asked = "curl -s -o /dev/null -m 5 -w '%{http_code}' -X POST \"$ORBITE_SERVER_AT/internal/think\"";
        // Without what the Agent is known by: what is run there beside it
        // is not given that to hold.
        self.ask(&[
            "exec",
            "-e",
            "ORBITE_SERVER_TOKEN=",
            &format!("orbite-agent-{id}"),
            "sh",
            "-c",
            asked,
        ])
        .is_ok_and(|status| status.trim() != "000" && !status.trim().is_empty())
    }

    /// Fills the volume of the Agent `id` with what is in `folder`, as the
    /// account the Agent runs as in its container.
    pub fn fill(&self, id: &str, folder: &Path) -> Result<(), String> {
        self.built()?;
        let named = format!("orbite-agent-{id}");
        self.ask(&["volume", "create", &named])?;
        let (to, from) = (format!("{named}:/to"), format!("{}:/from:ro", folder.display()));
        self.ask(&[
            "run",
            "--rm",
            "--user",
            "0",
            "--network",
            "none",
            "--cap-drop=ALL",
            "--cap-add=CHOWN",
            "--cap-add=DAC_OVERRIDE",
            "--cap-add=FOWNER",
            "--security-opt=no-new-privileges",
            "-e",
            "ORBITE_SERVER_TOKEN=held-by-a-server-and-being-moved-now",
            "-v",
            &to,
            "-v",
            &from,
            &self.image,
            "sh",
            "-c",
            "cp -a /from/. /to/ && chown -R orbite:orbite /to && chmod 700 /to",
        ])
        .map(|_| ())
    }

    fn ask(&self, asked: &[&str]) -> Result<String, String> {
        // Where the system keeps it, when asked as root: never one that
        // whoever ran this put ahead of it.
        let answered = std::process::Command::new(crate::system_program("docker"))
            .args(asked)
            .output()
            .map_err(|error| format!("docker could not be run: {error}"))?;
        if answered.status.success() {
            Ok(String::from_utf8_lossy(&answered.stdout).trim().to_owned())
        } else {
            Err(String::from_utf8_lossy(&answered.stderr).trim().to_owned())
        }
    }
}

impl Engine for Docker {
    fn seen_from_agents(&self) -> Option<std::net::IpAddr> {
        // Where the machine stands on the network its containers are on.
        self.ask(&["network", "inspect", "bridge", "-f", "{{(index .IPAM.Config 0).Gateway}}"])
            .ok()?
            .parse()
            .ok()
    }

    fn start(&self, id: &str, given: &[(String, String)]) -> Result<(), String> {
        self.built()?;
        let named = format!("orbite-agent-{id}");
        // There already, from this image and told the same: left as it is.
        // From another version, or told something else: made again, with
        // everything it kept, which is in its volume.
        let asked = "{{.Config.Image}}\n{{.State.Running}}\n{{json .Config.Env}}";
        if let Ok(there) = self.ask(&["inspect", "-f", asked, &named]) {
            let mut said = there.lines();
            let same = said.next() == Some(self.image.as_str()) && said.next() == Some("true");
            let told: Vec<String> =
                serde_json::from_str(said.next().unwrap_or("[]")).unwrap_or_default();
            // Told the same, no more and no less, of what a server tells.
            let said_of = |key: &str| {
                told.iter().find_map(|one| one.strip_prefix(&format!("{key}=")).map(str::to_owned))
            };
            let unchanged = [
                "ORBITE_SERVER_TOKEN",
                "ORBITE_AGENT_NAME",
                "ORBITE_SERVER_AT",
                "ORBITE_REACHED_AT",
            ]
            .iter()
            .all(|key| {
                said_of(key)
                    == given.iter().find(|(given, _)| given == key).map(|(_, value)| value.clone())
            });
            if same && unchanged {
                return Ok(());
            }
            self.ask(&["rm", "-f", &named])?;
        }
        self.ask(&["volume", "create", &named])?;
        // A network of its own: no Agent, and no other container of the
        // machine, is on the same wire as another Agent.
        match self.ask(&["network", "create", &named]) {
            Err(why) if !why.contains("already exists") => return Err(why),
            _ => {}
        }
        let kept = format!("{named}:/var/lib/orbite");
        let mut asked: Vec<String> = [
            "run",
            "-d",
            "--name",
            &named,
            "--restart=always",
            "--network",
            &named,
            "-v",
            &kept,
            // Nothing of the machine's own powers, none to be gained, and
            // bounds on what one Agent takes of it.
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--pids-limit=1024",
            "--memory=3g",
            // A browser keeps what its pages share in memory there.
            "--shm-size=1g",
            // The machine itself, as an Agent names it to reach the server.
            "--add-host=host.docker.internal:host-gateway",
        ]
        .map(String::from)
        .to_vec();
        // Given in the container's own surroundings, never on a command
        // line another account of the machine could read.
        let mut running = std::process::Command::new(crate::system_program("docker"));
        for (key, value) in given {
            asked.push("-e".into());
            asked.push(key.clone());
            running.env(key, value);
        }
        asked.push(self.image.clone());
        let answered = running
            .args(&asked)
            .output()
            .map_err(|error| format!("docker could not be run: {error}"))?;
        if answered.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&answered.stderr).trim().to_owned())
        }
    }

    fn reached_at(&self, id: &str) -> Result<String, String> {
        let address = self.ask(&[
            "inspect",
            "-f",
            &format!("{{{{(index .NetworkSettings.Networks \"orbite-agent-{id}\").IPAddress}}}}"),
            &format!("orbite-agent-{id}"),
        ])?;
        if address.is_empty() || address.parse::<std::net::IpAddr>().is_err() {
            return Err("its container is not running".into());
        }
        Ok(format!("http://{address}:8377"))
    }

    fn tidy(&self) {
        self.forget_other_versions();
    }

    fn remove(&self, id: &str) -> Result<(), String> {
        let named = format!("orbite-agent-{id}");
        // Gone already is gone; still there after this is not removed.
        let gone = |answered: Result<String, String>| match answered {
            Err(why) if !why.to_lowercase().contains("no such") => Err(why),
            _ => Ok(()),
        };
        gone(self.ask(&["rm", "-f", &named]))?;
        gone(self.ask(&["volume", "rm", "-f", &named]))?;
        gone(self.ask(&["network", "rm", &named]))
    }
}

struct Shared {
    home: PathBuf,
    database: Mutex<rusqlite::Connection>,
    engine: Box<dyn Engine>,
    http: reqwest::Client,
    /// The names the server is reached under, beside its address.
    names: Vec<String>,
    at_most: usize,
    /// Who every Agent thinks with, when the server holds the keys.
    thinks: Option<Thinks>,
    /// Where Agents reach the server from their containers.
    for_agents: Mutex<Option<String>>,
    /// Where the Owner's devices reach the server, when it is encrypted.
    reached_at: Option<String>,
    language: String,
    /// Held while the list of Agents is changed: one change at a time.
    changing: tokio::sync::Mutex<()>,
    /// How many things each Agent has the server thinking about at once.
    thinking: Mutex<std::collections::HashMap<String, usize>>,
    /// When each Agent last passed a word to another: only so many an hour.
    told: Mutex<std::collections::HashMap<String, Vec<std::time::Instant>>>,
    /// When each Agent last had a recording written down.
    heard: Mutex<std::collections::HashMap<String, Vec<std::time::Instant>>>,
    address: Mutex<Option<SocketAddr>>,
    /// What writes down what the Owner says aloud, when a Provider does.
    hears: Mutex<Option<Hears>>,
    /// What each Agent used of the Owner's Providers today: the day, then
    /// tokens by Agent. Kept in `spent.json`, to outlast a restart.
    spent: Mutex<(String, std::collections::HashMap<String, u64>)>,
}

/// Writes down what is said in a recording, with the keys the server holds.
pub type Hears = Arc<
    dyn Fn(PathBuf) -> std::pin::Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// The limits an Owner can put on what one Agent uses in a day, in tokens.
pub(crate) const DAILY_LIMITS: &[&str] = &["off", "500000", "2000000", "10000000"];

/// Asks the Owner's Providers what an Agent asks, with the keys the server
/// holds, and answers.
pub type Thinks = Arc<
    dyn Fn(
            crate::Asked,
        ) -> std::pin::Pin<
            Box<dyn Future<Output = Result<crate::Reply, crate::ProviderError>> + Send>,
        > + Send
        + Sync,
>;

impl Shared {
    /// What the Agent `id` used of the Owner's Providers today, in tokens.
    fn spent_today(&self, id: &str) -> u64 {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let spent = self.spent.lock().expect("not poisoned");
        if spent.0 == today { spent.1.get(id).copied().unwrap_or(0) } else { 0 }
    }

    /// Counts `tokens` more for the Agent `id` today.
    fn count(&self, id: &str, tokens: u64) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut spent = self.spent.lock().expect("not poisoned");
        if spent.0 != today {
            *spent = (today, std::collections::HashMap::new());
        }
        *spent.1.entry(id.to_owned()).or_insert(0) += tokens;
        if let Ok(written) = serde_json::to_string(&*spent) {
            let _ = std::fs::write(self.home.join("spent.json"), written);
        }
    }

    /// What an Agent is told when its container is made: what to trust the
    /// server by, what it is first called, where the server is, and where
    /// its Owner's devices reach it.
    fn given_to(&self, agent: &Held) -> Vec<(String, String)> {
        let mut given = vec![
            ("ORBITE_SERVER_TOKEN".to_owned(), agent.token.clone()),
            ("ORBITE_AGENT_NAME".to_owned(), agent.name.clone()),
        ];
        if let Some(at) = self.for_agents.lock().expect("not poisoned").clone() {
            given.push(("ORBITE_SERVER_AT".to_owned(), at));
        }
        if let Some(reached_at) = &self.reached_at {
            given.push(("ORBITE_REACHED_AT".to_owned(), format!("{reached_at}/a/{}", agent.id)));
        }
        given
    }

    /// The Agents of this server. Why not, when what is kept of them
    /// cannot be read: taken for none, the next one made would write over
    /// those that are there.
    fn agents(&self) -> io::Result<Vec<Held>> {
        match std::fs::read_to_string(self.home.join("agents.json")) {
            Ok(kept) => {
                let held: Vec<Held> = serde_json::from_str(&kept).map_err(io::Error::other)?;
                if held.iter().any(|agent| !is_an_id(&agent.id) || agent.token.len() < 32) {
                    return Err(io::Error::other("an agent is written down under no name of one"));
                }
                Ok(held)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    fn keep(&self, agents: &[Held]) -> io::Result<()> {
        use std::io::Write;
        let being_written = self.home.join("agents.writing");
        let _ = std::fs::remove_file(&being_written);
        let mut opening = std::fs::OpenOptions::new();
        opening.write(true).create_new(true);
        // Never readable by another account, not even for an instant.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opening.mode(0o600);
        }
        let mut file = opening.open(&being_written)?;
        file.write_all((serde_json::to_string_pretty(agents)? + "\n").as_bytes())?;
        file.sync_all()?;
        std::fs::rename(being_written, self.home.join("agents.json"))
    }
}

/// The server, serving.
pub struct Server {
    shared: Arc<Shared>,
    address: SocketAddr,
    serving: tokio::task::JoinHandle<()>,
    for_agents: Option<tokio::task::JoinHandle<()>>,
}

impl Server {
    /// Serves at `address` what is kept in `home`, running Agents with `engine`.
    pub async fn serving(
        home: &Path,
        address: SocketAddr,
        engine: impl Engine,
        names: Vec<String>,
        thinks: Option<Thinks>,
        for_agents_port: u16,
    ) -> io::Result<Self> {
        let seen = engine.seen_from_agents();
        let shared = Arc::new(Shared {
            home: home.to_owned(),
            database: Mutex::new(kept_in(home)?),
            engine: Box::new(engine),
            // What an Agent says as it goes is carried as it comes, for as
            // long as a device listens.
            // An Agent is asked where it is and nowhere else: what it
            // answers never sends the server anywhere.
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(io::Error::other)?,
            names,
            at_most: AGENTS_AT_MOST,
            thinks,
            for_agents: Mutex::new(None),
            reached_at: crate::Config::load(home)
                .ok()
                .and_then(|config| config.interface_reached_at)
                .filter(|reached| reached.starts_with("https://")),
            language: crate::Config::load(home)
                .ok()
                .and_then(|config| config.language)
                .unwrap_or_else(|| "en".into()),
            changing: tokio::sync::Mutex::new(()),
            thinking: Mutex::new(std::collections::HashMap::new()),
            told: Mutex::new(std::collections::HashMap::new()),
            heard: Mutex::new(std::collections::HashMap::new()),
            address: Mutex::new(None),
            hears: Mutex::new(None),
            spent: Mutex::new(
                std::fs::read_to_string(home.join("spent.json"))
                    .ok()
                    .and_then(|kept| serde_json::from_str(&kept).ok())
                    .unwrap_or_default(),
            ),
        });
        // A door of its own for the Agents, on the side of the machine
        // their containers see and nowhere else: what is asked there is
        // asked by an Agent, which shows what it was given.
        let mut for_agents = None;
        if shared.thinks.is_some() {
            // Without it an Agent could not think at all: better not to start.
            let seen = seen.ok_or_else(|| {
                io::Error::other("where the Agents' containers see this machine could not be found")
            })?;
            // Always the same port: an Agent is told once where the server
            // is, when it is made, and finds it there after every restart.
            let listening = tokio::net::TcpListener::bind((seen, for_agents_port)).await?;
            let port = listening.local_addr()?.port();
            let named =
                if seen.is_loopback() { seen.to_string() } else { "host.docker.internal".into() };
            *shared.for_agents.lock().expect("not poisoned") =
                Some(format!("http://{named}:{port}"));
            let asked = Router::new()
                .route("/internal/think", post(think))
                .route("/internal/ask/{what}", post(asked_by_an_agent))
                .layer(axum::extract::DefaultBodyLimit::max(THOUGHT_AT_MOST))
                .layer(axum::middleware::from_fn_with_state(shared.clone(), by_an_agent_only))
                .with_state(shared.clone());
            for_agents = Some(tokio::spawn(async move {
                let _ = axum::serve(listening, asked).await;
            }));
        }
        // The Agents there already are started again as this version runs
        // them: made anew from its image when they were of another, which
        // takes the time to build it once. They answer as they were meanwhile.
        let told_where = shared.clone();
        let again = shared.clone();
        tokio::task::spawn_blocking(move || {
            for agent in again.agents().unwrap_or_default() {
                if let Err(why) = again.engine.start(&agent.id, &again.given_to(&agent)) {
                    eprintln!("The agent {} could not be started: {why}", agent.name);
                }
            }
            again.engine.tidy();
        });
        // The interface's own page, served by the server: what it asks of
        // an Agent is asked of the one chosen there.
        let routes = crate::interface::pages()
            .route("/api/pair", post(pair))
            .route("/api/me", get(me))
            .route("/api/agents", get(agents).post(create))
            .route("/api/agents/{id}", axum::routing::delete(remove))
            .route("/api/devices", get(devices).post(one_more_device))
            .route("/api/devices/{name}", axum::routing::delete(turn_away))
            .route("/a/{id}/signed-in", get(signed_in))
            .route("/a/{id}/{*rest}", any(carry))
            .layer(axum::middleware::from_fn_with_state(shared.clone(), asked_by_its_name))
            .with_state(shared);
        let listening = tokio::net::TcpListener::bind(address).await?;
        let address = listening.local_addr()?;
        *told_where.address.lock().expect("not poisoned") = Some(address);
        let serving = tokio::spawn(async move {
            let _ = axum::serve(listening, routes).await;
        });
        Ok(Self { shared: told_where, address, serving, for_agents })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Writes down what the Owner says aloud to any Agent with `hears`.
    pub fn hearing_with(self, hears: Option<Hears>) -> Self {
        *self.shared.hears.lock().expect("not poisoned") = hears;
        self
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.serving.abort();
        if let Some(for_agents) = &self.for_agents {
            for_agents.abort();
        }
    }
}

async fn pair(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    if !from_the_page(&headers, &shared.names) {
        return refused(StatusCode::FORBIDDEN, "not from the interface");
    }
    // Reached encrypted: said by what carried the request, or known of
    // where the server is reached, as behind its own door.
    let_in(&shared.database, &sent, came_encrypted(&headers) || shared.reached_at.is_some()).await
}

/// Says whether this device is let in, and that this is a server: the page
/// then asks which Agents it holds.
async fn me(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    match device_in(&shared.database, &headers) {
        Some(device) => {
            Json(json!({ "device": device, "language": shared.language, "server": true }))
                .into_response()
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "not paired", "language": shared.language, "server": true })),
        )
            .into_response(),
    }
}

/// Answers only what was asked of the server by a name of its own, as the
/// interface of one Agent does.
async fn asked_by_its_name(
    State(shared): State<Arc<Shared>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let asked = request.headers().get(header::HOST).and_then(|host| host.to_str().ok());
    if !crate::interface::is_a_name_of_its_own(asked, &shared.names) {
        return refused(StatusCode::MISDIRECTED_REQUEST, "not this place");
    }
    next.run(request).await
}

/// Where a service sends the Owner back once they signed in for one Agent:
/// that Agent is told, and the page that says how it went is the server's
/// own, never one an Agent wrote. Whoever comes here is known by what the
/// service gave them, which the Agent checks: a device is not asked for,
/// since a browser coming back from another site shows none.
async fn signed_in(
    State(shared): State<Arc<Shared>>,
    Named(id): Named<String>,
    axum::extract::RawQuery(asked): axum::extract::RawQuery,
) -> Response {
    let french = shared.language == "fr";
    let agent = shared.agents().ok().and_then(|held| held.into_iter().find(|agent| agent.id == id));
    let mut went = None;
    if let Some(agent) = agent {
        let engine = shared.clone();
        let wanted = agent.id.clone();
        if let Ok(Ok(at)) =
            tokio::task::spawn_blocking(move || engine.engine.reached_at(&wanted)).await
            && let Ok(answer) = shared
                .http
                .get(format!("{at}/signed-in?{}", asked.unwrap_or_default()))
                .header("x-orbite-server", &agent.token)
                .send()
                .await
            && let Ok(told) = answer.json::<Value>().await
        {
            went = Some((
                told["well"].as_bool().unwrap_or(false),
                told["said"].as_str().unwrap_or_default().chars().take(200).collect::<String>(),
            ));
        }
    }
    let (well, said) = went.unwrap_or_else(|| {
        let none =
            if french { "La connexion n'a pas abouti." } else { "The sign-in did not go through." };
        (false, none.to_owned())
    });
    crate::interface::sign_in_page(french, well, &said)
}

/// The devices let in to this server, for one of them.
async fn devices(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    let Some(asking) = device_in(&shared.database, &headers) else {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    };
    match crate::interface_devices(&shared.home) {
        Ok(all) => {
            let all: Vec<Value> = all
                .iter()
                .map(|device| {
                    json!({ "name": device.name, "seen": device.seen, "this": device.name == asking })
                })
                .collect();
            Json(json!({ "devices": all })).into_response()
        }
        Err(_) => refused(StatusCode::INTERNAL_SERVER_ERROR, "the devices could not be read"),
    }
}

/// A code that lets one more device in, with where it is typed and the
/// picture a phone's camera reads: given to a device already let in, which
/// shows it to the Owner. Never to an Agent.
async fn one_more_device(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !from_the_page(&headers, &shared.names) || device_in(&shared.database, &headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let Ok(code) = crate::pairing_code(&shared.home) else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "no code could be made");
    };
    let at = shared
        .reached_at
        .clone()
        // Where it listens is where it is reached only when that is one place.
        .or_else(|| {
            shared
                .address
                .lock()
                .expect("not poisoned")
                .filter(|at| !at.ip().is_unspecified())
                .map(|at| format!("http://{at}"))
        });
    let link = at.as_ref().map(|at| format!("{at}/#{code}"));
    let picture = link.as_deref().and_then(crate::code_picture);
    Json(json!({ "code": code, "at": at, "link": link, "picture": picture })).into_response()
}

/// Turns a device away, asked by another that is let in.
async fn turn_away(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Named(name): Named<String>,
) -> Response {
    if !from_the_page(&headers, &shared.names) || device_in(&shared.database, &headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    match crate::turn_away(&shared.home, &name) {
        Ok(gone) => Json(json!({ "gone": gone })).into_response(),
        Err(_) => refused(StatusCode::INTERNAL_SERVER_ERROR, "it could not be turned away"),
    }
}

/// The Agents of this server, for a device let in.
async fn agents(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if device_in(&shared.database, &headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let Ok(held) = shared.agents() else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "the agents could not be read");
    };
    let listed: Vec<Value> =
        held.iter().map(|agent| json!({ "id": agent.id, "name": agent.name })).collect();
    Json(json!({ "agents": listed, "at_most": shared.at_most })).into_response()
}

/// Makes one more Agent, called as the Owner says.
async fn create(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    if !from_the_page(&headers, &shared.names) || device_in(&shared.database, &headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let name: String = sent["name"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|letter| !letter.is_control())
        .take(40)
        .collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return refused(StatusCode::BAD_REQUEST, "what is it called");
    }
    // One change to the Agents at a time.
    let _changing = shared.changing.lock().await;
    let Ok(mut held) = shared.agents() else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "the agents could not be read");
    };
    if held.len() >= shared.at_most {
        return refused(StatusCode::CONFLICT, "this server holds no more agents");
    }
    // Agents are told apart, and written to, by their names.
    if held.iter().any(|other| other.name.to_lowercase() == name.to_lowercase()) {
        return refused(StatusCode::CONFLICT, "another agent is called that already");
    }
    // Its own for good: never known by what another Agent already is.
    let Ok(agent) = Held::new(&name, &held) else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "nothing random could be made");
    };
    let (id, given) = (agent.id.clone(), shared.given_to(&agent));
    // Kept before it is started: a container without its Agent written
    // down would be one nobody can remove.
    held.push(agent);
    if shared.keep(&held).is_err() {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "it could not be written down");
    }
    let engine = shared.clone();
    let started = tokio::task::spawn_blocking(move || engine.engine.start(&id, &given)).await;
    match started {
        Ok(Ok(())) => {
            let made = held.last().expect("just added");
            Json(json!({ "id": made.id, "name": made.name })).into_response()
        }
        problem => {
            // Nothing is left of what did not start.
            let gone = held.pop().expect("just added");
            let _ = shared.keep(&held);
            let cleaning = shared.clone();
            let _ = tokio::task::spawn_blocking(move || cleaning.engine.remove(&gone.id)).await;
            let why = match problem {
                Ok(Err(why)) => why,
                _ => "it could not be started".into(),
            };
            refused(StatusCode::BAD_GATEWAY, &why)
        }
    }
}

/// Removes an Agent and everything it kept.
async fn remove(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Named(id): Named<String>,
) -> Response {
    if !from_the_page(&headers, &shared.names) || device_in(&shared.database, &headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let _changing = shared.changing.lock().await;
    let Ok(mut held) = shared.agents() else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "the agents could not be read");
    };
    let Some(place) = held.iter().position(|agent| agent.id == id) else {
        return refused(StatusCode::NOT_FOUND, "no such agent");
    };
    let gone = held.remove(place);
    let engine = shared.clone();
    let removed = tokio::task::spawn_blocking(move || engine.engine.remove(&gone.id)).await;
    // Still written down while it is still there: it can be removed again.
    if !matches!(removed, Ok(Ok(()))) {
        return refused(StatusCode::BAD_GATEWAY, "it could not be removed");
    }
    if shared.keep(&held).is_err() {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "it could not be written down");
    }
    Json(json!({ "removed": true })).into_response()
}

/// Carries what a device asks of one Agent to that Agent, and its answer
/// back, as it comes. The Agent is told which device this is by the
/// server, which it trusts; it is never shown what a device is known by.
async fn carry(
    State(shared): State<Arc<Shared>>,
    Named((id, rest)): Named<(String, String)>,
    request: axum::extract::Request,
) -> Response {
    let headers = request.headers().clone();
    let Some(device) = device_in(&shared.database, &headers) else {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    };
    let method = request.method().clone();
    // What changes something comes from the server's own page or app.
    let from_here = from_the_page(&headers, &shared.names);
    if method != axum::http::Method::GET && !from_here {
        return refused(StatusCode::FORBIDDEN, "not from the interface");
    }
    let Some(agent) =
        shared.agents().ok().and_then(|held| held.into_iter().find(|agent| agent.id == id))
    else {
        return refused(StatusCode::NOT_FOUND, "no such agent");
    };
    let engine = shared.clone();
    let wanted = agent.id.clone();
    let Ok(Ok(at)) = tokio::task::spawn_blocking(move || engine.engine.reached_at(&wanted)).await
    else {
        return refused(StatusCode::BAD_GATEWAY, "this agent is not running");
    };
    // A path is a path: nothing in it passes for what is asked with it.
    if rest.contains(['?', '#']) {
        return refused(StatusCode::BAD_REQUEST, "not a path");
    }
    let asked = request.uri().query().map(|asked| format!("?{asked}")).unwrap_or_default();
    let Ok(body) = axum::body::to_bytes(request.into_body(), 64 * 1024 * 1024).await else {
        return refused(StatusCode::PAYLOAD_TOO_LARGE, "too much at once");
    };
    // A name another Agent has is not given to this one.
    let renamed = serde_json::from_slice::<Value>(&body)
        .ok()
        .filter(|_| rest == "api/settings" && method == axum::http::Method::POST)
        .filter(|sent| sent["key"] == "name")
        .and_then(|sent| sent["value"].as_str().map(|name| name.trim().to_owned()))
        .filter(|name| !name.is_empty());
    if let Some(named) = &renamed
        && shared
            .agents()
            .unwrap_or_default()
            .iter()
            .any(|other| other.id != agent.id && other.name.to_lowercase() == named.to_lowercase())
    {
        return refused(StatusCode::CONFLICT, "another agent is called that already");
    }
    // Who the Agents think with is the server's: the model is set here,
    // for all of them, from the settings of any of them.
    let of_settings = rest == "api/settings";
    if of_settings
        && method == axum::http::Method::POST
        && let Ok(sent) = serde_json::from_slice::<Value>(&body)
        && let (Some(key @ ("model" | "light" | "daily_limit")), Some(value)) =
            (sent["key"].as_str(), sent["value"].as_str())
    {
        let (home, key, value) = (shared.home.clone(), key.to_owned(), value.trim().to_owned());
        // How much this one Agent may use in a day is written with it.
        if key == "daily_limit" {
            if !DAILY_LIMITS.contains(&value.as_str()) {
                return refused(StatusCode::BAD_REQUEST, "not a limit there is");
            }
            let _changing = shared.changing.lock().await;
            let Ok(mut held) = shared.agents() else {
                return refused(StatusCode::INTERNAL_SERVER_ERROR, "the agents could not be read");
            };
            if let Some(one) = held.iter_mut().find(|one| one.id == agent.id) {
                one.daily_tokens = value.parse().ok();
            }
            return match shared.keep(&held) {
                Ok(()) => Json(json!({ "set": true })).into_response(),
                Err(_) => refused(StatusCode::INTERNAL_SERVER_ERROR, "it could not be kept"),
            };
        }
        if value.is_empty() || value.contains(char::is_whitespace) || value.chars().count() > 200 {
            return refused(StatusCode::BAD_REQUEST, "not a value for it");
        }
        let set = tokio::task::spawn_blocking(move || {
            let store =
                crate::Store::open(home.join("orbite.db")).map_err(|error| error.to_string())?;
            crate::Settings::of(&store, Some(&home)).set(&key, &value)
        })
        .await;
        return match set {
            Ok(Ok(())) => Json(json!({ "set": true })).into_response(),
            Ok(Err(why)) => refused(StatusCode::BAD_REQUEST, &why),
            Err(_) => refused(StatusCode::INTERNAL_SERVER_ERROR, "it could not be kept"),
        };
    }
    let mut carrying = shared
        .http
        .request(method.clone(), format!("{at}/{rest}{asked}"))
        .header("x-orbite-server", &agent.token)
        .header(
            "x-orbite-device",
            device.replace(|letter: char| !letter.is_ascii() || letter.is_control(), "?"),
        )
        .body(body);
    // Said to come from the interface's own page only when it did.
    if from_here {
        carrying = carrying.header("x-orbite", "1");
    }
    // What says how to read the body, and where a device left off; nothing
    // a device is known by, and nothing of where it came from.
    for kept in
        [header::CONTENT_TYPE, header::ACCEPT, header::HeaderName::from_static("last-event-id")]
    {
        if let Some(value) = headers.get(&kept) {
            carrying = carrying.header(kept, value);
        }
    }
    let Ok(answer) = carrying.send().await else {
        return refused(StatusCode::BAD_GATEWAY, "this agent does not answer");
    };
    // Renamed by its Owner, an Agent is known to the server by its new
    // name: the others write to it by that one.
    if of_settings
        && method == axum::http::Method::POST
        && answer.status().is_success()
        && let Some(named) = renamed
    {
        let _changing = shared.changing.lock().await;
        if let Ok(mut held) = shared.agents() {
            if let Some(one) = held.iter_mut().find(|one| one.id == agent.id) {
                one.name = named;
            }
            let _ = shared.keep(&held);
        }
    }
    // An Agent's settings are shown with what is the server's among them:
    // the models there are to choose from, and the ones in force.
    if of_settings && method == axum::http::Method::GET && answer.status().is_success() {
        let Ok(mut told) = answer.json::<Value>().await else {
            return refused(StatusCode::BAD_GATEWAY, "its settings could not be read");
        };
        let (models, usual) = crate::interface::models_to_choose(&shared.home).await;
        told["models"] = json!(models);
        told["usual"] = json!(usual);
        let home = shared.home.clone();
        let in_force = tokio::task::spawn_blocking(move || {
            let store = crate::Store::open(home.join("orbite.db")).ok()?;
            let settings = crate::Settings::of(&store, Some(&home));
            Some((settings.get("model").ok()?, settings.get("light").ok()?))
        })
        .await
        .ok()
        .flatten();
        if let (Some((model, light)), Some(all)) = (in_force, told["settings"].as_array_mut()) {
            for (key, value) in [("model", model), ("light", light)] {
                match all.iter_mut().find(|one| one["key"] == key) {
                    Some(one) => one["value"] = json!(value),
                    None => all.push(json!({ "key": key, "value": value, "choices": [] })),
                }
            }
        }
        // And what this Agent may use in a day, with what it used today.
        if let Some(all) = told["settings"].as_array_mut() {
            let limit = agent.daily_tokens.map_or("off".to_owned(), |limit| limit.to_string());
            all.retain(|one| one["key"] != "daily_limit");
            all.push(json!({ "key": "daily_limit", "value": limit, "choices": DAILY_LIMITS }));
        }
        told["spent_today"] = json!(shared.spent_today(&agent.id));
        return Json(told).into_response();
    }
    // What an Agent answers is shown under the server's own name, where a
    // device is let in to every Agent. So it is never a page: only what an
    // Agent's interface is known to answer, and nothing a browser would run.
    let kind = answer
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|kind| kind.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    let harmless =
        ["application/json", "text/event-stream", "application/octet-stream", "text/plain"]
            .iter()
            .any(|known| kind.starts_with(known))
            || [
                "image/png",
                "image/jpeg",
                "image/gif",
                "image/webp",
                "audio/",
                "video/",
                "application/pdf",
            ]
            .iter()
            .any(|known| kind.starts_with(known));
    let mut back = Response::builder()
        .status(answer.status())
        .header(
            header::CONTENT_TYPE,
            if harmless { kind.as_str() } else { "application/octet-stream" },
        )
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'");
    for kept in [header::CACHE_CONTROL, header::CONTENT_DISPOSITION] {
        if let Some(value) = answer.headers().get(&kept) {
            back = back.header(kept, value);
        }
    }
    // A device turned away while it listens is told nothing more.
    let (still, known_by) = (shared.clone(), headers.clone());
    let mut looked = std::time::Instant::now();
    let told = tokio_stream::StreamExt::take_while(answer.bytes_stream(), move |_| {
        if looked.elapsed() < std::time::Duration::from_secs(5) {
            return true;
        }
        looked = std::time::Instant::now();
        device_in(&still.database, &known_by).is_some()
    });
    back.body(Body::from_stream(told))
        .unwrap_or_else(|_| refused(StatusCode::BAD_GATEWAY, "its answer could not be carried"))
}

/// The Agent that asks, known by what it shows: one of the server's own.
fn agent_asking(shared: &Shared, headers: &HeaderMap) -> Option<Held> {
    let shown = headers.get("x-orbite-agent").and_then(|shown| shown.to_str().ok())?;
    // Compared as what they hash to: how long it takes says nothing.
    let shown = crate::interface::hashed(shown);
    shared.agents().ok()?.into_iter().find(|agent| crate::interface::hashed(&agent.token) == shown)
}

/// How many words one Agent passes to others in an hour, at the most: two
/// Agents answering each other without end stop here.
const WORDS_AN_HOUR: usize = 12;

/// How many recordings one Agent has written down in an hour, at the most.
const RECORDINGS_AN_HOUR: usize = 40;

/// The longest word one Agent passes to another.
const WORD_AT_MOST: usize = 4000;

/// Where the Agent `id` is reached, asked of the engine without holding
/// anything else up.
async fn reached(shared: &Arc<Shared>, id: &str) -> Result<String, String> {
    let (engine, id) = (shared.clone(), id.to_owned());
    tokio::task::spawn_blocking(move || engine.engine.reached_at(&id))
        .await
        .unwrap_or_else(|_| Err("the engine could not be asked".into()))
}

/// Lets through to the Agents' door only what one of the server's own
/// Agents asks: whoever else knocks is turned away before it is listened to.
async fn by_an_agent_only(
    State(shared): State<Arc<Shared>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if agent_asking(&shared, request.headers()).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not an agent of this server");
    }
    next.run(request).await
}

/// What an Agent asks of the server beside thinking: who the other Agents
/// are, to pass one of them a word, to have a recording written down.
/// Nothing here lets a device in or turns one away: that is asked by a
/// device already let in, of the server itself, and never by an Agent,
/// which something it read may have misled.
async fn asked_by_an_agent(
    State(shared): State<Arc<Shared>>,
    Named(what): Named<String>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    let Some(asking) = agent_asking(&shared, &headers) else {
        return refused(StatusCode::UNAUTHORIZED, "not an agent of this server");
    };
    match what.as_str() {
        // By the names the server knows them by, which only their Owner
        // changes: an Agent does not pass for another by calling itself so.
        "agents" => {
            let names: Vec<String> = shared
                .agents()
                .unwrap_or_default()
                .into_iter()
                .filter(|other| other.id != asking.id)
                .map(|other| other.name)
                .collect();
            Json(json!({ "agents": names })).into_response()
        }
        "tell" => {
            let (to, text) = (
                sent["to"].as_str().unwrap_or_default().trim().to_lowercase(),
                sent["text"].as_str().unwrap_or_default().trim(),
            );
            if text.is_empty() || text.chars().count() > WORD_AT_MOST {
                return refused(StatusCode::BAD_REQUEST, "a word of a few lines, no more");
            }
            let found = shared
                .agents()
                .unwrap_or_default()
                .into_iter()
                .find(|other| other.id != asking.id && other.name.to_lowercase() == to);
            let Some(other) = found else {
                return refused(StatusCode::NOT_FOUND, "no other agent is called that");
            };
            {
                let mut told = shared.told.lock().expect("not poisoned");
                let lately = told.entry(asking.id.clone()).or_default();
                lately.retain(|when| when.elapsed() < std::time::Duration::from_secs(3600));
                if lately.len() >= WORDS_AN_HOUR {
                    return refused(
                        StatusCode::TOO_MANY_REQUESTS,
                        "too many words passed to other agents this hour: wait, or tell your Owner",
                    );
                }
                lately.push(std::time::Instant::now());
            }
            let Ok(at) = reached(&shared, &other.id).await else {
                return refused(StatusCode::BAD_GATEWAY, "that agent is not running");
            };
            let passed = shared
                .http
                .post(format!("{at}/api/from-agent"))
                .header("x-orbite-server", &other.token)
                .json(&json!({ "from": asking.name, "text": text }))
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await;
            match passed {
                Ok(taken) if taken.status().is_success() => {
                    Json(json!({ "passed": true })).into_response()
                }
                _ => refused(StatusCode::BAD_GATEWAY, "that agent did not take it"),
            }
        }
        // What the Owner said aloud to this Agent, written down with the
        // keys the server holds.
        "hear" => {
            use base64::Engine as _;
            // Not without end, and not once the day's limit is used up:
            // writing down is paid for with the Owner's key too.
            {
                let mut heard = shared.heard.lock().expect("not poisoned");
                let lately = heard.entry(asking.id.clone()).or_default();
                lately.retain(|when| when.elapsed() < std::time::Duration::from_secs(3600));
                let spent = asking
                    .daily_tokens
                    .is_some_and(|limit| shared.spent_today(&asking.id) >= limit);
                if lately.len() >= RECORDINGS_AN_HOUR || spent {
                    return refused(StatusCode::TOO_MANY_REQUESTS, "too many recordings for now");
                }
                lately.push(std::time::Instant::now());
            }
            let Some(hears) = shared.hears.lock().expect("not poisoned").clone() else {
                return refused(
                    StatusCode::NOT_IMPLEMENTED,
                    "none of your Owner's AI providers writes recordings down",
                );
            };
            let ending: String = sent["name"]
                .as_str()
                .unwrap_or_default()
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(5)
                .collect();
            let content = sent["content"]
                .as_str()
                .and_then(|content| base64::engine::general_purpose::STANDARD.decode(content).ok());
            let (Some(content), Ok(folder)) = (content, tempfile::tempdir()) else {
                return refused(StatusCode::BAD_REQUEST, "not a recording");
            };
            // Under a name of the server's own: nothing an Agent says is a path here.
            let recording = folder.path().join(format!("recording.{ending}"));
            if std::fs::write(&recording, content).is_err() {
                return refused(StatusCode::INTERNAL_SERVER_ERROR, "it could not be put down");
            }
            match hears(recording).await {
                Ok(text) => Json(json!({ "text": text })).into_response(),
                // Why not is the server's to know: what a Provider says of
                // a key may say the key.
                Err(why) => {
                    eprintln!("A recording could not be written down for {}: {why}", asking.name);
                    refused(StatusCode::UNPROCESSABLE_ENTITY, "it could not be written down")
                }
            }
        }
        _ => refused(StatusCode::NOT_FOUND, "nothing an agent asks"),
    }
}

/// Thinks for an Agent: asked by one of the server's own Agents, which
/// shows what it was given, the Owner's Providers are asked with the keys
/// the server holds, and what it cost is counted for all of them.
async fn think(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    let Some(asking) = agent_asking(&shared, &headers) else {
        return refused(StatusCode::UNAUTHORIZED, "not an agent of this server");
    };
    let (Some(thinks), Some(asked)) = (&shared.thinks, crate::Asked::read(&sent)) else {
        return refused(StatusCode::BAD_REQUEST, "not what an agent asks");
    };
    // Nor more in a day than its Owner allows it.
    if let Some(limit) = asking.daily_tokens
        && shared.spent_today(&asking.id) >= limit
    {
        return Json(crate::served::written(&Err(crate::ProviderError::Busy {
            reason: format!("this agent {}", crate::served::SPENT_FOR_TODAY),
            retry_after: Some(std::time::Duration::from_secs(3600)),
        })))
        .into_response();
    }
    // One Agent does not take all of the server's thinking for itself.
    {
        let mut thinking = shared.thinking.lock().expect("not poisoned");
        let under_way = thinking.entry(asking.id.clone()).or_insert(0);
        if *under_way >= THOUGHTS_AT_ONCE {
            return Json(crate::served::written(&Err(crate::ProviderError::Busy {
                reason: "this agent already has the server thinking about several things".into(),
                retry_after: Some(std::time::Duration::from_secs(5)),
            })))
            .into_response();
        }
        *under_way += 1;
    }
    // Counted down however this ends: an Agent that stopped waiting for a
    // thought is not left with it forever under way.
    struct UnderWay(Arc<Shared>, String);
    impl Drop for UnderWay {
        fn drop(&mut self) {
            if let Some(under_way) = self.0.thinking.lock().expect("not poisoned").get_mut(&self.1)
            {
                *under_way = under_way.saturating_sub(1);
            }
        }
    }
    let counted = UnderWay(shared.clone(), asking.id.clone());
    let answered = thinks(asked).await;
    drop(counted);
    if let Ok(crate::Reply { usage: Some(usage), .. }) = &answered {
        shared.count(&asking.id, usage.input + usage.output);
    }
    if let Ok(crate::Reply { usage: Some(usage), .. }) = &answered
        && let Ok(store) = crate::Store::open(shared.home.join("orbite.db"))
    {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let _ = store.record_usage(&today, usage);
    }
    // Why a Provider refused its key is the server's to know, not an Agent's:
    // what a Provider says of a key may say the key.
    let answered = answered.map_err(|problem| match problem {
        crate::ProviderError::Rejected(_) => crate::ProviderError::Rejected(
            "the server's Provider refused its key or plan: your Owner sets it on the server"
                .into(),
        ),
        other => other,
    });
    Json(crate::served::written(&answered)).into_response()
}

/// A few random lowercase letters and digits.
fn random(length: usize) -> io::Result<String> {
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut random = vec![0u8; length];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    Ok(random.iter().map(|byte| LETTERS[usize::from(*byte) % LETTERS.len()] as char).collect())
}
