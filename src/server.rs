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
        let named = format!("orbite-agent-{id}");
        self.ask(&["volume", "create", &named])?;
        // A network of its own: no Agent, and no other container of the
        // machine, is on the same wire as another Agent.
        self.ask(&["network", "create", &named])?;
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
    let (Ok(mut id), Ok(token)) = (random(6), random(32)) else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "nothing random could be made");
    };
    // Its own for good: never one that another Agent already has.
    while held.iter().any(|agent| agent.id == id) {
        match random(6) {
            Ok(other) => id = other,
            Err(_) => {
                return refused(StatusCode::INTERNAL_SERVER_ERROR, "nothing random could be made");
            }
        }
    }
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
