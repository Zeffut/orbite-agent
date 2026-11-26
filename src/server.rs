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
    token: String,
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
}

/// Docker, asked through its own program.
pub struct Docker {
    /// The image an Agent runs from.
    pub image: String,
}

impl Docker {
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
        let (container, volume) = (format!("orbite-agent-{id}"), format!("orbite-agent-{id}"));
        self.ask(&["volume", "create", &volume])?;
        let kept = format!("{volume}:/var/lib/orbite");
        let mut asked: Vec<String> =
            ["run", "-d", "--name", &container, "--restart=always", "-v", &kept]
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
            "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
            &format!("orbite-agent-{id}"),
        ])?;
        if address.is_empty() {
            return Err("its container is not running".into());
        }
        Ok(format!("http://{address}:8377"))
    }

    fn remove(&self, id: &str) -> Result<(), String> {
        let named = format!("orbite-agent-{id}");
        // Gone already is gone.
        let _ = self.ask(&["rm", "-f", &named]);
        let _ = self.ask(&["volume", "rm", "-f", &named]);
        Ok(())
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
    fn agents(&self) -> Vec<Held> {
        std::fs::read_to_string(self.home.join("agents.json"))
            .ok()
            .and_then(|kept| serde_json::from_str(&kept).ok())
            .unwrap_or_default()
    }

    fn keep(&self, agents: &[Held]) -> io::Result<()> {
        let being_written = self.home.join("agents.writing");
        std::fs::write(&being_written, serde_json::to_string_pretty(agents)? + "\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&being_written, std::fs::Permissions::from_mode(0o600))?;
        }
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
    ) -> io::Result<Self> {
        let seen = engine.seen_from_agents();
        let shared = Arc::new(Shared {
            home: home.to_owned(),
            database: Mutex::new(kept_in(home)?),
            engine: Box::new(engine),
            // What an Agent says as it goes is carried as it comes, for as
            // long as a device listens.
            http: reqwest::Client::builder().build().map_err(io::Error::other)?,
            names,
            at_most: AGENTS_AT_MOST,
            thinks,
            for_agents: Mutex::new(None),
        });
        // A door of its own for the Agents, on the side of the machine
        // their containers see and nowhere else: what is asked there is
        // asked by an Agent, which shows what it was given.
        let mut for_agents = None;
        if let (Some(seen), true) = (seen, shared.thinks.is_some()) {
            let listening = tokio::net::TcpListener::bind((seen, 0)).await?;
            *shared.for_agents.lock().expect("not poisoned") =
                Some(format!("http://{}", listening.local_addr()?));
            let asked = Router::new()
                .route("/internal/think", post(think))
                .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
                .with_state(shared.clone());
            for_agents = Some(tokio::spawn(async move {
                let _ = axum::serve(listening, asked).await;
            }));
        }
        let routes = Router::new()
            .route("/api/pair", post(pair))
            .route("/api/agents", get(agents).post(create))
            .route("/api/agents/{id}", axum::routing::delete(remove))
            .route("/a/{id}/{*rest}", any(carry))
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

/// The Agents of this server, for a device let in.
async fn agents(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if device_in(&shared.database, &headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let listed: Vec<Value> =
        shared.agents().iter().map(|agent| json!({ "id": agent.id, "name": agent.name })).collect();
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
    let mut held = shared.agents();
    if held.len() >= shared.at_most {
        return refused(StatusCode::CONFLICT, "this server holds no more agents");
    }
    let (Ok(id), Ok(token)) = (random(6), random(32)) else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "nothing random could be made");
    };
    let agent = Held { id: id.clone(), name: name.clone(), token: token.clone() };
    // Kept before it is started: a container without its Agent written
    // down would be one nobody can remove.
    held.push(agent);
    if shared.keep(&held).is_err() {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "it could not be written down");
    }
    let mut given = vec![
        ("ORBITE_SERVER_TOKEN".to_owned(), token.clone()),
        ("ORBITE_AGENT_NAME".to_owned(), name.clone()),
    ];
    if let Some(at) = shared.for_agents.lock().expect("not poisoned").clone() {
        given.push(("ORBITE_SERVER_AT".to_owned(), at));
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
    let mut held = shared.agents();
    let Some(place) = held.iter().position(|agent| agent.id == id) else {
        return refused(StatusCode::NOT_FOUND, "no such agent");
    };
    let gone = held.remove(place);
    let engine = shared.clone();
    let removed = tokio::task::spawn_blocking(move || engine.engine.remove(&gone.id)).await;
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
    if method != axum::http::Method::GET && !from_the_page(&headers, &shared.names) {
        return refused(StatusCode::FORBIDDEN, "not from the interface");
    }
    let Some(agent) = shared.agents().into_iter().find(|agent| agent.id == id) else {
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
        .header("x-orbite", "1")
        .body(body);
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
    let mut back = Response::builder().status(answer.status());
    for kept in [header::CONTENT_TYPE, header::CACHE_CONTROL, header::CONTENT_DISPOSITION] {
        if let Some(value) = answer.headers().get(&kept) {
            back = back.header(kept, value);
        }
    }
    back.body(Body::from_stream(answer.bytes_stream()))
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
    let known = shown.is_some_and(|shown| {
        // Compared as what they hash to: how long it takes says nothing.
        let shown = crate::interface::hashed(shown);
        shared.agents().iter().any(|agent| crate::interface::hashed(&agent.token) == shown)
    });
    if !known {
        return refused(StatusCode::UNAUTHORIZED, "not an agent of this server");
    }
    let (Some(thinks), Some(asked)) = (&shared.thinks, crate::Asked::read(&sent)) else {
        return refused(StatusCode::BAD_REQUEST, "not what an agent asks");
    };
    let answered = thinks(asked).await;
    if let Ok(crate::Reply { usage: Some(usage), .. }) = &answered
        && let Ok(store) = crate::Store::open(shared.home.join("orbite.db"))
    {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let _ = store.record_usage(&today, usage);
    }
    Json(crate::served::written(&answered)).into_response()
}

/// A few random lowercase letters and digits.
fn random(length: usize) -> io::Result<String> {
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut random = vec![0u8; length];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    Ok(random.iter().map(|byte| LETTERS[usize::from(*byte) % LETTERS.len()] as char).collect())
}
