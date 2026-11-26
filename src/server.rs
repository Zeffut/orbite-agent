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
}

impl Held {
    /// One more Agent, called `name`, known by nothing another is.
    pub(crate) fn new(name: &str, others: &[Held]) -> io::Result<Self> {
        let mut id = random(6)?;
        while others.iter().any(|other| other.id == id) {
            id = random(6)?;
        }
        Ok(Self { id, name: name.to_owned(), token: random(32)? })
    }
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
        let answered = std::process::Command::new("docker")
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
            if same && given.iter().all(|(key, value)| told.contains(&format!("{key}={value}"))) {
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
        let mut running = std::process::Command::new("docker");
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
}

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
            Ok(kept) => serde_json::from_str(&kept).map_err(io::Error::other),
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
                .layer(axum::extract::DefaultBodyLimit::max(THOUGHT_AT_MOST))
                .with_state(shared.clone());
            for_agents = Some(tokio::spawn(async move {
                let _ = axum::serve(listening, asked).await;
            }));
        }
        // The Agents there already are started again as this version runs
        // them: made anew from its image when they were of another, which
        // takes the time to build it once. They answer as they were meanwhile.
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
            .route("/a/{id}/signed-in", get(signed_in))
            .route("/a/{id}/{*rest}", any(carry))
            .layer(axum::middleware::from_fn_with_state(shared.clone(), asked_by_its_name))
            .with_state(shared);
        let listening = tokio::net::TcpListener::bind(address).await?;
        let address = listening.local_addr()?;
        let serving = tokio::spawn(async move {
            let _ = axum::serve(listening, routes).await;
        });
        Ok(Self { address, serving, for_agents })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
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
    let_in(&shared.database, &sent, came_encrypted(&headers)).await
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
    let asked = request.uri().query().map(|asked| format!("?{asked}")).unwrap_or_default();
    let Ok(body) = axum::body::to_bytes(request.into_body(), 64 * 1024 * 1024).await else {
        return refused(StatusCode::PAYLOAD_TOO_LARGE, "too much at once");
    };
    let mut carrying = shared
        .http
        .request(method, format!("{at}/{rest}{asked}"))
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

/// Thinks for an Agent: asked by one of the server's own Agents, which
/// shows what it was given, the Owner's Providers are asked with the keys
/// the server holds, and what it cost is counted for all of them.
async fn think(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    let shown = headers.get("x-orbite-agent").and_then(|shown| shown.to_str().ok());
    let asking = shown.and_then(|shown| {
        // Compared as what they hash to: how long it takes says nothing.
        let shown = crate::interface::hashed(shown);
        shared
            .agents()
            .ok()?
            .into_iter()
            .find(|agent| crate::interface::hashed(&agent.token) == shown)
    });
    let Some(asking) = asking else {
        return refused(StatusCode::UNAUTHORIZED, "not an agent of this server");
    };
    let (Some(thinks), Some(asked)) = (&shared.thinks, crate::Asked::read(&sent)) else {
        return refused(StatusCode::BAD_REQUEST, "not what an agent asks");
    };
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
    let answered = thinks(asked).await;
    if let Some(under_way) = shared.thinking.lock().expect("not poisoned").get_mut(&asking.id) {
        *under_way = under_way.saturating_sub(1);
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
