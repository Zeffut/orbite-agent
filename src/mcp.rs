//! MCP servers: programs on the Host, or services elsewhere, whose tools
//! become Actions of the Agent. They are listed in `mcp.json`, in the form
//! other MCP clients use.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::secrets::Secrets;
use crate::{ActionRequest, ActionSpec, Approval};

/// The version of the protocol Orbite speaks to servers of today: each
/// request stands alone and says who asks.
const PROTOCOL: &str = "2026-07-28";

/// The version it speaks to servers written before, which are greeted first.
const EARLIER_PROTOCOL: &str = "2025-06-18";

/// How long a local server is given to say which of the two it speaks.
const PROBE_LIMIT: Duration = Duration::from_secs(20);

/// How long a server is given to start and say what it offers.
const START_LIMIT: Duration = Duration::from_secs(60);

/// How long a server is given to carry out one call.
const CALL_LIMIT: Duration = Duration::from_secs(120);

/// The longest message taken from a server, on one line or in one answer.
const LINE_LIMIT: usize = 8_000_000;

/// How many pages of its list of tools a server is asked for, and how many
/// tools are taken from it.
const PAGES_LIMIT: usize = 50;
const TOOLS_LIMIT: usize = 500;

/// The longest description of a tool passed on to the model.
const DESCRIPTION_LIMIT: usize = 1_000;

/// One server as `mcp.json` describes it: a command, or an address.
#[derive(Deserialize, Clone)]
struct Described {
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
struct Settings {
    #[serde(default, alias = "servers")]
    #[serde(rename = "mcpServers")]
    servers: BTreeMap<String, Described>,
}

/// One tool of a server, as an Action.
#[derive(Clone)]
struct Tool {
    /// Its name as an Action of the Agent.
    action: String,
    /// Its name for its server.
    name: String,
    title: String,
    description: String,
    input: Value,
    only_reads: bool,
    destroys: bool,
}

struct Server {
    name: String,
    described: Described,
    /// The live connection, once made.
    link: tokio::sync::Mutex<Option<Link>>,
    /// What it offers, or why it could not be asked; None before it was tried.
    tools: std::sync::Mutex<Option<Result<Vec<Tool>, String>>>,
}

pub(crate) struct Mcp {
    servers: Vec<Server>,
    secrets: Secrets,
    /// What is wrong with `mcp.json`, when it cannot be read.
    problem: Option<String>,
    http: reqwest::Client,
}

impl Mcp {
    /// The servers listed in `home`'s `mcp.json`, none of them started yet.
    pub(crate) fn configured_in(home: &Path) -> Self {
        let file: PathBuf = home.join("mcp.json");
        let (settings, problem) = match std::fs::read_to_string(&file) {
            Err(_) => (Settings::default(), None),
            Ok(written) => match serde_json::from_str::<Settings>(&written) {
                Ok(settings) => (settings, None),
                Err(error) => {
                    (Settings::default(), Some(format!("mcp.json could not be read: {error}")))
                }
            },
        };
        let servers = settings
            .servers
            .into_iter()
            .map(|(name, described)| Server {
                name,
                described,
                link: tokio::sync::Mutex::new(None),
                tools: std::sync::Mutex::new(None),
            })
            .collect();
        Self { servers, secrets: Secrets::kept_in(home), problem, http: reqwest::Client::new() }
    }

    /// Starts every server not tried yet, all at once, and asks each what it
    /// offers. With `again`, those that failed are tried once more.
    pub(crate) async fn connect(&self, again: bool) {
        let waiting = self.servers.iter().filter(|server| {
            let tools = server.tools.lock().unwrap();
            tools.is_none() || (again && matches!(*tools, Some(Err(_))))
        });
        let starting = waiting.map(|server| async move {
            let offered = match self.start(server).await {
                Ok((link, tools)) => {
                    *server.link.lock().await = Some(link);
                    Ok(tools)
                }
                Err(problem) => Err(problem),
            };
            *server.tools.lock().unwrap() = Some(offered);
        });
        futures_all(starting).await;
    }

    /// A fresh connection to `server`, greeted, and what it offers.
    async fn start(&self, server: &Server) -> Result<(Link, Vec<Tool>), String> {
        match tokio::time::timeout(START_LIMIT, self.greet(server)).await {
            Ok(started) => started,
            Err(_) => Err(format!("it did not answer within {} seconds", START_LIMIT.as_secs())),
        }
    }

    async fn greet(&self, server: &Server) -> Result<(Link, Vec<Tool>), String> {
        let mut link = self.open(&server.described)?;
        if !link.speaks_todays_protocol().await? {
            let hello = json!({
                "protocolVersion": EARLIER_PROTOCOL,
                "capabilities": {},
                "clientInfo": { "name": "orbite", "version": env!("CARGO_PKG_VERSION") },
            });
            link.ask("initialize", hello).await?;
            link.tell("notifications/initialized").await?;
        }
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            let page = link
                .ask("tools/list", cursor.map_or(json!({}), |cursor| json!({ "cursor": cursor })))
                .await?;
            for tool in page["tools"].as_array().into_iter().flatten() {
                let Some(name) = tool["name"].as_str() else { continue };
                let notes = &tool["annotations"];
                let only_reads = notes["readOnlyHint"].as_bool().unwrap_or(false);
                let description = tool["description"].as_str().unwrap_or_default();
                tools.push(Tool {
                    action: format!("{}_{}", plain(&server.name), plain(name)),
                    name: name.to_owned(),
                    title: tool["title"]
                        .as_str()
                        .or(notes["title"].as_str())
                        .unwrap_or_default()
                        .to_owned(),
                    description: description.chars().take(DESCRIPTION_LIMIT).collect(),
                    input: match &tool["inputSchema"] {
                        Value::Object(schema) => Value::Object(schema.clone()),
                        _ => json!({ "type": "object", "properties": {} }),
                    },
                    only_reads,
                    // Unless a tool says otherwise, what it does cannot be undone.
                    destroys: !only_reads && notes["destructiveHint"].as_bool().unwrap_or(true),
                });
            }
            cursor = page["nextCursor"].as_str().map(str::to_owned);
            pages += 1;
            // A server that never runs out of pages is not listened to for ever.
            if cursor.is_none() || pages >= PAGES_LIMIT || tools.len() >= TOOLS_LIMIT {
                break;
            }
        }
        Ok((link, tools))
    }

    fn open(&self, described: &Described) -> Result<Link, String> {
        if let Some(url) = &described.url {
            let headers = described
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), self.filled(value)))
                .collect();
            return Ok(Link::Remote {
                http: self.http.clone(),
                url: url.clone(),
                headers,
                session: None,
                next: 0,
                modern: false,
            });
        }
        let Some(command) = &described.command else {
            return Err("it has neither a command nor a url".into());
        };
        let mut starting = Command::new(command);
        starting
            .args(&described.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            // A group of its own, so that what it starts in turn (as npx
            // and uvx do) is stopped with it.
            .process_group(0)
            .kill_on_drop(true);
        // What is Orbite's own stays with Orbite.
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("ORBITE_") {
                starting.env_remove(name);
            }
        }
        for (name, value) in &described.env {
            starting.env(name, self.filled(value));
        }
        let mut child =
            starting.spawn().map_err(|error| format!("{command} could not be started: {error}"))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(format!("{command} could not be talked to"));
        };
        Ok(Link::Local {
            group: child.id(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next: 0,
            modern: false,
        })
    }

    /// `text` with each ${NAME} replaced by the secret of that name.
    fn filled(&self, text: &str) -> String {
        let mut filled = String::new();
        let mut rest = text;
        while let Some((before, after)) = rest.split_once("${") {
            filled.push_str(before);
            match after.split_once('}') {
                Some((name, after)) => {
                    // Orbite's own keys are not given to a server.
                    if !name.starts_with("ORBITE_") {
                        filled.push_str(&self.secrets.get(name).ok().flatten().unwrap_or_default());
                    }
                    rest = after;
                }
                None => {
                    filled.push_str("${");
                    rest = after;
                }
            }
        }
        filled + rest
    }

    /// The Actions the servers add, as told to the model, without any that
    /// would take the name of one in `taken`.
    pub(crate) fn specs(&self, taken: &[ActionSpec]) -> Vec<ActionSpec> {
        let mut specs: Vec<ActionSpec> = Vec::new();
        for server in &self.servers {
            let Some(Ok(tools)) = &*server.tools.lock().unwrap() else { continue };
            for tool in tools {
                if taken.iter().chain(&specs).any(|other| other.name == tool.action) {
                    continue;
                }
                specs.push(ActionSpec {
                    name: tool.action.clone(),
                    description: tool.description.clone(),
                    input: tool.input.clone(),
                });
            }
        }
        specs
    }

    fn find(&self, request: &ActionRequest) -> Option<(&Server, Tool)> {
        self.servers.iter().find_map(|server| {
            let tools = server.tools.lock().unwrap();
            let tool =
                tools.as_ref()?.as_ref().ok()?.iter().find(|tool| tool.action == request.name)?;
            Some((server, tool.clone()))
        })
    }

    /// What the Owner is asked before `request`, when it is for a server's
    /// tool: None inside when the tool only reads.
    pub(crate) fn review(&self, request: &ActionRequest) -> Option<Option<Approval>> {
        let (server, tool) = self.find(request)?;
        if tool.only_reads {
            return Some(None);
        }
        let what =
            if tool.title.is_empty() { one_sentence(&tool.description) } else { &tool.title };
        let what = if what.is_empty() { &tool.name } else { what };
        Some(Some(Approval {
            summary: format!("{what} ({})", server.name),
            detail: serde_json::to_string_pretty(&request.input).unwrap_or_default(),
            irreversible: tool.destroys,
            critical: false,
            contained: false,
        }))
    }

    /// Carries out `request` when it is for a server's tool; the text is its Outcome.
    pub(crate) async fn carry_out(&self, request: &ActionRequest) -> Option<String> {
        let (server, tool) = self.find(request)?;
        let mut link = server.link.lock().await;
        if link.is_none() {
            // The server was lost on an earlier call: it is started afresh.
            match self.start(server).await {
                Ok((fresh, _)) => *link = Some(fresh),
                Err(problem) => {
                    return Some(format!(
                        "The server {} could not be reached: {problem}.",
                        server.name
                    ));
                }
            }
        }
        let call = json!({ "name": tool.name, "arguments": request.input });
        let asked = link.as_mut()?.ask("tools/call", call);
        let answer = match tokio::time::timeout(CALL_LIMIT, asked).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(problem)) => {
                *link = None;
                return Some(format!("The server {} could not do it: {problem}.", server.name));
            }
            Err(_) => {
                *link = None;
                return Some(format!(
                    "The server {} did not answer within {} seconds.",
                    server.name,
                    CALL_LIMIT.as_secs()
                ));
            }
        };
        if answer["resultType"] == "input_required" {
            return Some(format!(
                "The server {} asks for more before doing this, which Orbite cannot give it yet.",
                server.name
            ));
        }
        let mut said: Vec<String> = Vec::new();
        for part in answer["content"].as_array().into_iter().flatten() {
            match part["type"].as_str() {
                Some("text") => said.push(part["text"].as_str().unwrap_or_default().to_owned()),
                Some("resource") => {
                    said.push(part["resource"]["text"].as_str().unwrap_or("[a file]").to_owned())
                }
                Some(other) => said.push(format!("[{other}]")),
                None => {}
            }
        }
        if said.is_empty() && !answer["structuredContent"].is_null() {
            said.push(answer["structuredContent"].to_string());
        }
        let said = if said.is_empty() { "Done.".to_owned() } else { said.join("\n") };
        Some(if answer["isError"].as_bool().unwrap_or(false) {
            format!("The tool failed: {said}")
        } else {
            said
        })
    }

    /// Each server and how many tools it offers, or why it offers none.
    pub(crate) fn status(&self) -> (Vec<ServerState>, Option<String>) {
        let servers = self
            .servers
            .iter()
            .map(|server| {
                let state = match &*server.tools.lock().unwrap() {
                    None => Err("not started yet".to_owned()),
                    Some(Ok(tools)) => Ok(tools.len()),
                    Some(Err(problem)) => Err(problem.clone()),
                };
                (server.name.clone(), state)
            })
            .collect();
        (servers, self.problem.clone())
    }
}

/// A server by name, and how many tools it offers or why it offers none.
pub(crate) type ServerState = (String, Result<usize, String>);

/// A connection to one server.
#[allow(clippy::large_enum_variant)]
enum Link {
    Local {
        child: Child,
        /// The group of programs the server leads, to stop them all.
        group: Option<u32>,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
        next: u64,
        /// True when the server speaks today's protocol.
        modern: bool,
    },
    Remote {
        http: reqwest::Client,
        url: String,
        headers: Vec<(String, String)>,
        session: Option<String>,
        next: u64,
        modern: bool,
    },
}

impl Drop for Link {
    fn drop(&mut self) {
        // Everything the server started goes with it.
        if let Link::Local { group: Some(group), .. } = self {
            // SAFETY: a signal sent to the group of a program this one started.
            unsafe { libc::kill(-(*group as libc::pid_t), libc::SIGKILL) };
        }
    }
}

/// What a server answered: its result, or its error with the error's code.
type Answered = Result<Value, (i64, String)>;

impl Link {
    /// Finds out whether the server speaks today's protocol, and remembers
    /// it; false for one that has to be greeted first, as before.
    async fn speaks_todays_protocol(&mut self) -> Result<bool, String> {
        *self.modern_mut() = true;
        let local = matches!(self, Link::Local { .. });
        let probe = self.exchange("server/discover", json!({}));
        let probed = if local {
            match tokio::time::timeout(PROBE_LIMIT, probe).await {
                Ok(probed) => probed,
                // No answer at all is how some earlier servers take a
                // request they do not know.
                Err(_) => Ok(Err((0, String::new()))),
            }
        } else {
            probe.await
        };
        let speaks = match probed {
            Ok(Ok(_)) => true,
            Ok(Err((code, problem))) if code == UNSUPPORTED_VERSION => return Err(problem),
            Ok(Err(_)) => false,
            // Refused for who asks, not for how it is asked.
            Err(problem) if !local && problem.contains("refused Orbite") => return Err(problem),
            Err(problem) if local => return Err(problem),
            Err(_) => false,
        };
        *self.modern_mut() = speaks;
        Ok(speaks)
    }

    fn modern_mut(&mut self) -> &mut bool {
        match self {
            Link::Local { modern, .. } | Link::Remote { modern, .. } => modern,
        }
    }

    /// Asks the server for `method` and waits for its result.
    async fn ask(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.exchange(method, params).await?.map_err(|(_, problem)| problem)
    }

    /// Asks the server for `method`; the outer error is a failure to talk.
    async fn exchange(&mut self, method: &str, mut params: Value) -> Result<Answered, String> {
        if *self.modern_mut() {
            // Each request says which protocol it speaks and who asks.
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion": PROTOCOL,
                "io.modelcontextprotocol/clientInfo": { "name": "orbite", "version": env!("CARGO_PKG_VERSION") },
                "io.modelcontextprotocol/clientCapabilities": {},
            });
        }
        match self {
            Link::Local { stdin, stdout, next, child, .. } => {
                *next += 1;
                let id = *next;
                let asked =
                    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
                send(stdin, &asked).await?;
                loop {
                    // One message a line, and no line longer than a
                    // server has any reason to write.
                    let mut line = Vec::new();
                    let mut limited = (&mut *stdout).take(LINE_LIMIT as u64 + 1);
                    let reading = limited.read_until(b'\n', &mut line);
                    // A server that ended may have left a program of its own
                    // holding its output open: its own end is what counts.
                    let read = tokio::select! {
                        read = reading => read.map_err(|error| error.to_string())?,
                        _ = child.wait() => 0,
                    };
                    if read == 0 {
                        return Err("it stopped before answering".into());
                    }
                    if line.len() > LINE_LIMIT {
                        return Err(format!(
                            "it wrote a message of more than {} MB",
                            LINE_LIMIT / 1_000_000
                        ));
                    }
                    let line = String::from_utf8_lossy(&line);
                    let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                    if message["id"] == json!(id) && message["method"].is_null() {
                        return Ok(answered(&message));
                    }
                    // What the server asks of Orbite is declined: it offers nothing back.
                    if !message["id"].is_null() && message["method"].is_string() {
                        let reply = match message["method"].as_str() {
                            Some("ping") => {
                                json!({ "jsonrpc": "2.0", "id": message["id"], "result": {} })
                            }
                            _ => json!({ "jsonrpc": "2.0", "id": message["id"],
                                "error": { "code": -32601, "message": "Method not found" } }),
                        };
                        send(stdin, &reply).await?;
                    }
                }
            }
            Link::Remote { next, .. } => {
                *next += 1;
                let id = *next;
                let asked =
                    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
                let body = self.post(&asked).await?;
                // The answer is plain, or the last of a stream of events.
                if let Ok(message) = serde_json::from_str::<Value>(&body) {
                    return Ok(answered(&message));
                }
                body.lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
                    .find(|message| message["id"] == json!(id))
                    .map_or(Err("its answer was not understood".into()), |message| {
                        Ok(answered(&message))
                    })
            }
        }
    }

    /// Tells the server of `method`, expecting no answer.
    async fn tell(&mut self, method: &str) -> Result<(), String> {
        let told = json!({ "jsonrpc": "2.0", "method": method });
        match self {
            Link::Local { stdin, .. } => send(stdin, &told).await,
            Link::Remote { .. } => self.post(&told).await.map(|_| ()),
        }
    }

    async fn post(&mut self, message: &Value) -> Result<String, String> {
        let Link::Remote { http, url, headers, session, modern, .. } = self else {
            return Err("this server is not reached that way".into());
        };
        let mut sending = http
            .post(url.as_str())
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", if *modern { PROTOCOL } else { EARLIER_PROTOCOL })
            .timeout(CALL_LIMIT)
            .json(message);
        if *modern {
            // Today's protocol repeats in headers what the request is for.
            if let Some(method) = message["method"].as_str() {
                sending = sending.header("mcp-method", method);
            }
            if let Some(name) = message["params"]["name"].as_str() {
                sending = sending.header("mcp-name", in_a_header(name));
            }
        }
        for (name, value) in headers.iter() {
            sending = sending.header(name, value);
        }
        if let Some(session) = session {
            sending = sending.header("mcp-session-id", session.as_str());
        }
        let answer = sending
            .send()
            .await
            .map_err(|error| format!("it could not be reached: {}", error.without_url()))?;
        if let Some(given) =
            answer.headers().get("mcp-session-id").and_then(|given| given.to_str().ok())
        {
            *session = Some(given.to_owned());
        }
        let status = answer.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(format!(
                "it refused Orbite (HTTP {}): it needs a key, or to be signed in to",
                status.as_u16()
            ));
        }
        // Read as it comes, and no further than a server has reason to send.
        let mut answer = answer;
        let mut read: Vec<u8> = Vec::new();
        loop {
            match answer.chunk().await {
                Ok(Some(part)) => read.extend_from_slice(&part),
                Ok(None) => break,
                Err(error) => {
                    return Err(format!(
                        "its answer did not arrive whole: {}",
                        error.without_url()
                    ));
                }
            }
            if read.len() > LINE_LIMIT {
                return Err(format!("it answered with more than {} MB", LINE_LIMIT / 1_000_000));
            }
        }
        let body = String::from_utf8_lossy(&read).into_owned();
        // An error told in the protocol's own words is an answer.
        if !status.is_success() && !body.contains("\"jsonrpc\"") {
            return Err(format!("it answered HTTP {}", status.as_u16()));
        }
        Ok(body)
    }
}

/// The error of a server that speaks today's protocol, but not Orbite's version of it.
const UNSUPPORTED_VERSION: i64 = -32022;

/// `value` as a header carries it: as it is when plain, else encoded.
fn in_a_header(value: &str) -> String {
    use base64::Engine;
    let plain =
        value.bytes().all(|byte| (0x20..0x7f).contains(&byte)) && !value.starts_with("=?base64?");
    if plain {
        value.to_owned()
    } else {
        format!("=?base64?{}?=", base64::engine::general_purpose::STANDARD.encode(value))
    }
}

async fn send(stdin: &mut ChildStdin, message: &Value) -> Result<(), String> {
    let line = format!("{message}\n");
    stdin.write_all(line.as_bytes()).await.map_err(|_| "it stopped listening".to_owned())?;
    stdin.flush().await.map_err(|_| "it stopped listening".to_owned())
}

/// The result in a server's answer, or the error it gave.
fn answered(message: &Value) -> Answered {
    let error = &message["error"];
    if error.is_null() {
        return Ok(message["result"].clone());
    }
    let problem = error["message"].as_str().map_or_else(|| error.to_string(), str::to_owned);
    Err((error["code"].as_i64().unwrap_or(0), problem))
}

/// `name` in the letters every model accepts in the name of an Action.
fn plain(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect()
}

/// The first sentence of `text`, without its full stop.
fn one_sentence(text: &str) -> &str {
    text.split_once(". ").map_or(text, |(first, _)| first).trim_end_matches('.')
}

/// Runs all of `tasks` at once and waits for the last.
async fn futures_all(tasks: impl Iterator<Item = impl Future<Output = ()>>) {
    let mut pending: Vec<_> = tasks.map(Box::pin).collect();
    std::future::poll_fn(|context| {
        pending.retain_mut(|task| task.as_mut().poll(context).is_pending());
        if pending.is_empty() { std::task::Poll::Ready(()) } else { std::task::Poll::Pending }
    })
    .await;
}
