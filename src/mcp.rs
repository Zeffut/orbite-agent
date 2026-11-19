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
    /// A client the Owner registered with the service themselves, for one
    /// that takes no program it does not already know.
    client: Option<OwnClient>,
}

#[derive(Deserialize, Clone)]
struct OwnClient {
    id: String,
    /// Usually ${NAME}: a secret, never written here.
    secret: Option<String>,
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
    /// The sign-in the Owner is in the middle of, when there is one.
    signing_in: std::sync::Mutex<Option<SigningIn>>,
}

/// Where a sign-in ends: an address of the Host itself, where nothing
/// listens. The Owner's browser stops there, and they send the address back.
pub(crate) const SIGN_IN_ENDS_AT: &str = "http://127.0.0.1:8976/callback";

/// A sign-in under way: what is needed to finish it once the Owner sends
/// back the address their browser ended on.
struct SigningIn {
    server: String,
    /// What the challenge in the link was made from; it never leaves the Host.
    verifier: String,
    state: String,
    client: String,
    exchange: String,
    issuer: String,
    resource: String,
}

/// What a sign-in obtained, kept sealed.
#[derive(serde::Serialize, Deserialize)]
struct SignedIn {
    access: String,
    #[serde(default)]
    refresh: Option<String>,
    /// Seconds since 1970 after which `access` is no longer taken.
    #[serde(default)]
    until: Option<u64>,
    client: String,
    exchange: String,
    resource: String,
}

/// A server, one of its tools, and what the tool is given: nothing when
/// the model only asks what the tool takes.
type Aimed<'a> = (&'a Server, Tool, Option<Value>);

/// How much a server may say about its tools, in bytes, before they are
/// offered as one Action: about three thousand tokens.
const HEAVY: usize = 12_000;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Random letters, for what must not be guessed.
fn unguessable() -> Result<String, String> {
    use base64::Engine;
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|error| error.to_string())?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random))
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
        Self {
            servers,
            secrets: Secrets::kept_in(home),
            problem,
            http: reqwest::Client::new(),
            signing_in: std::sync::Mutex::new(None),
        }
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
        self.renew(server).await;
        match tokio::time::timeout(START_LIMIT, self.greet(server)).await {
            Ok(started) => started,
            Err(_) => Err(format!("it did not answer within {} seconds", START_LIMIT.as_secs())),
        }
    }

    async fn greet(&self, server: &Server) -> Result<(Link, Vec<Tool>), String> {
        let mut link = self.open(server)?;
        let speaks = match link.speaks_todays_protocol().await {
            Ok(speaks) => speaks,
            // Some earlier servers quit at a request they do not know: a
            // local one is started again and greeted the earlier way.
            Err(_) if server.described.command.is_some() => {
                link = self.open(server)?;
                false
            }
            Err(problem) => return Err(problem),
        };
        if !speaks {
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

    fn open(&self, server: &Server) -> Result<Link, String> {
        let described = &server.described;
        if let Some(url) = &described.url {
            let mut headers: Vec<(String, String)> = described
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), self.filled(value)))
                .collect();
            // Signed in to: what was obtained then is shown on every request.
            if let Some(signed) = self.signed_in(&server.name) {
                headers.push(("authorization".into(), format!("Bearer {}", signed.access)));
            }
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

    /// Under which name what a sign-in to `server` obtained is sealed: one of
    /// Orbite's own, which the Agent neither lists nor uses.
    fn kept_as(server: &str) -> String {
        format!("ORBITE_MCP_{}", plain(server).to_uppercase())
    }

    /// The secret of the client the Owner brought for `server`, when it has one.
    fn client_secret(&self, server: &str) -> Option<String> {
        let own =
            self.servers.iter().find(|known| known.name == server)?.described.client.as_ref()?;
        own.secret.as_ref().map(|secret| self.filled(secret)).filter(|secret| !secret.is_empty())
    }

    fn signed_in(&self, server: &str) -> Option<SignedIn> {
        serde_json::from_str(&self.secrets.get(&Self::kept_as(server)).ok()??).ok()
    }

    /// Obtains a fresh access when the one kept is about to end, with what
    /// the sign-in gave for that.
    async fn renew(&self, server: &Server) {
        let Some(signed) = self.signed_in(&server.name) else { return };
        let (Some(until), Some(refresh)) = (signed.until, signed.refresh.clone()) else { return };
        if until > now() + 60 {
            return;
        }
        let secret = self.client_secret(&server.name);
        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", signed.client.as_str()),
            ("resource", signed.resource.as_str()),
        ];
        if let Some(secret) = &secret {
            form.push(("client_secret", secret.as_str()));
        }
        let Ok(answer) = self
            .http
            .post(&signed.exchange)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form_written(&form))
            .timeout(Duration::from_secs(30))
            .send()
            .await
        else {
            return;
        };
        let Ok(given) = answer.json::<Value>().await else { return };
        let Some(access) = given["access_token"].as_str() else { return };
        let renewed = SignedIn {
            access: access.to_owned(),
            refresh: given["refresh_token"].as_str().map(str::to_owned).or(Some(refresh)),
            until: given["expires_in"].as_u64().map(|lasts| now() + lasts),
            ..signed
        };
        if let Ok(written) = serde_json::to_string(&renewed) {
            let _ = self.secrets.set(&Self::kept_as(&server.name), &written);
        }
    }

    /// Begins signing in to the server called `name`: the text is the
    /// address the Owner follows.
    pub(crate) async fn sign_in(&self, name: &str) -> Result<String, String> {
        let server = self
            .servers
            .iter()
            .find(|server| server.name == name)
            .ok_or("there is no server of that name")?;
        let address = server
            .described
            .url
            .clone()
            .ok_or("it is a program on this machine: there is nothing to sign in to")?;
        let wrong =
            |error: reqwest::Error| format!("it could not be reached: {}", error.without_url());
        let fetch = |at: String| async move {
            let answer = self.http.get(&at).timeout(Duration::from_secs(20)).send().await.ok()?;
            if !answer.status().is_success() {
                return None;
            }
            answer.json::<Value>().await.ok()
        };
        // Where the server says to learn how it is signed in to.
        let asked = self
            .http
            .post(&address)
            .json(&json!({}))
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(wrong)?;
        let told = asked
            .headers()
            .get("www-authenticate")
            .and_then(|told| told.to_str().ok())
            .and_then(|told| told.split("resource_metadata=\"").nth(1))
            .and_then(|rest| rest.split('"').next())
            .map(str::to_owned);
        let at = url::Url::parse(&address).map_err(|_| "its address is not one")?;
        let origin = at.origin().ascii_serialization();
        let path = at.path().trim_end_matches('/');
        let mut about = None;
        for place in told.into_iter().chain([
            format!("{origin}/.well-known/oauth-protected-resource{path}"),
            format!("{origin}/.well-known/oauth-protected-resource"),
        ]) {
            about = fetch(place).await;
            if about.is_some() {
                break;
            }
        }
        let about = about.ok_or("it does not say how it is signed in to: it may take a key instead, in its headers in mcp.json")?;
        let issuer = about["authorization_servers"][0]
            .as_str()
            .ok_or("it names nowhere to sign in")?
            .trim_end_matches('/')
            .to_owned();
        let resource = about["resource"].as_str().unwrap_or(&address).to_owned();
        let issued =
            url::Url::parse(&issuer).map_err(|_| "where it says to sign in is not an address")?;
        let (base, inside) =
            (issued.origin().ascii_serialization(), issued.path().trim_end_matches('/').to_owned());
        let mut how = None;
        for place in [
            format!("{base}/.well-known/oauth-authorization-server{inside}"),
            format!("{base}/.well-known/openid-configuration{inside}"),
            format!("{issuer}/.well-known/openid-configuration"),
        ] {
            // Only the place that names itself as the one asked for is believed.
            how = fetch(place).await.filter(|how| {
                how["issuer"].as_str().is_some_and(|named| named.trim_end_matches('/') == issuer)
            });
            if how.is_some() {
                break;
            }
        }
        let how = how.ok_or("where it says to sign in does not describe itself")?;
        let (Some(entrance), Some(exchange)) =
            (how["authorization_endpoint"].as_str(), how["token_endpoint"].as_str())
        else {
            return Err("where it says to sign in does not say how".into());
        };
        let proven = how["code_challenge_methods_supported"]
            .as_array()
            .is_some_and(|ways| ways.iter().any(|way| way == "S256"));
        if !proven {
            return Err("its sign-in does not protect what Orbite would be handed: not used".into());
        }
        // With the client the Owner brought; else Orbite introduces itself,
        // where newcomers are taken.
        let client = match &server.described.client {
            Some(own) => self.filled(&own.id),
            None => self.introduced_at(how["registration_endpoint"].as_str()).await?,
        };

        let (verifier, state) = (unguessable()?, unguessable()?);
        let challenge = {
            use base64::Engine;
            use sha2::{Digest, Sha256};
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()))
        };
        let mut link =
            url::Url::parse(entrance).map_err(|_| "where it says to sign in is not an address")?;
        {
            let mut asked = link.query_pairs_mut();
            asked
                .append_pair("response_type", "code")
                .append_pair("client_id", &client)
                .append_pair("redirect_uri", SIGN_IN_ENDS_AT)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("state", &state)
                .append_pair("resource", &resource);
            if let Some(scopes) = about["scopes_supported"].as_array() {
                let scopes: Vec<&str> = scopes.iter().filter_map(Value::as_str).collect();
                if !scopes.is_empty() {
                    asked.append_pair("scope", &scopes.join(" "));
                }
            }
        }
        *self.signing_in.lock().unwrap() = Some(SigningIn {
            server: name.to_owned(),
            verifier,
            state,
            client,
            exchange: exchange.to_owned(),
            issuer,
            resource,
        });
        Ok(link.to_string())
    }

    /// Introduces Orbite to a sign-in that takes newcomers at `registry`:
    /// the name it is given there.
    async fn introduced_at(&self, registry: Option<&str>) -> Result<String, String> {
        let Some(registry) = registry else {
            return Err("its sign-in takes no program it does not already know. If it offers a key or a client of \
                        your own, put the key in its headers in mcp.json instead"
                .into());
        };
        let myself = json!({
            "client_name": "Orbite",
            "redirect_uris": [SIGN_IN_ENDS_AT],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "application_type": "native",
        });
        let known = self
            .http
            .post(registry)
            .json(&myself)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|error| format!("it could not be reached: {}", error.without_url()))?;
        let known = known
            .json::<Value>()
            .await
            .map_err(|error| format!("it could not be reached: {}", error.without_url()))?;
        let client = known["client_id"]
            .as_str()
            .ok_or("its sign-in did not take Orbite as a newcomer")?
            .to_owned();
        Ok(client)
    }

    /// True when `text` is the address a sign-in under way ended on.
    pub(crate) fn is_signing_in(&self, text: &str) -> bool {
        text.trim().starts_with(SIGN_IN_ENDS_AT) && self.signing_in.lock().unwrap().is_some()
    }

    /// Finishes the sign-in under way with the address the Owner's browser
    /// ended on: the server's name and how many tools it now offers.
    pub(crate) async fn signed_in_at(&self, ended_on: &str) -> Result<(String, usize), String> {
        let Some(begun) = self.signing_in.lock().unwrap().take() else {
            return Err("no sign-in is under way".into());
        };
        let again = format!("start again with /mcp connect {}", begun.server);
        let ended = url::Url::parse(ended_on.trim())
            .map_err(|_| format!("that is not an address: {again}"))?;
        let given: std::collections::HashMap<String, String> =
            ended.query_pairs().into_owned().collect();
        // Only the answer to the very link that was sent is taken.
        if given.get("state") != Some(&begun.state) {
            return Err(format!("that address is not the answer to the link I sent: {again}"));
        }
        if given.get("iss").is_some_and(|issuer| issuer.trim_end_matches('/') != begun.issuer) {
            return Err(format!("that address does not come from where the link led: {again}"));
        }
        let Some(code) = given.get("code") else {
            let refused =
                given.get("error_description").or(given.get("error")).cloned().unwrap_or_default();
            return Err(format!("the sign-in was not granted ({refused}): {again}"));
        };
        let secret = self.client_secret(&begun.server);
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", SIGN_IN_ENDS_AT),
            ("client_id", begun.client.as_str()),
            ("code_verifier", begun.verifier.as_str()),
            ("resource", begun.resource.as_str()),
        ];
        if let Some(secret) = &secret {
            form.push(("client_secret", secret.as_str()));
        }
        let answer = self
            .http
            .post(&begun.exchange)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form_written(&form))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|error| {
                format!("the sign-in could not be finished ({}): {again}", error.without_url())
            })?;
        let given = answer.json::<Value>().await.unwrap_or_default();
        let Some(access) = given["access_token"].as_str() else {
            return Err(format!("the sign-in was not accepted: {again}"));
        };
        let signed = SignedIn {
            access: access.to_owned(),
            refresh: given["refresh_token"].as_str().map(str::to_owned),
            until: given["expires_in"].as_u64().map(|lasts| now() + lasts),
            client: begun.client,
            exchange: begun.exchange,
            resource: begun.resource,
        };
        let written = serde_json::to_string(&signed).map_err(|error| error.to_string())?;
        self.secrets
            .set(&Self::kept_as(&begun.server), &written)
            .map_err(|error| error.to_string())?;
        // Connected afresh, now that it lets Orbite in.
        if let Some(server) = self.servers.iter().find(|server| server.name == begun.server) {
            *server.link.lock().await = None;
            *server.tools.lock().unwrap() = None;
        }
        self.connect(false).await;
        let tools = self
            .status()
            .0
            .into_iter()
            .find(|(name, _)| *name == begun.server)
            .map(|(_, state)| state);
        match tools {
            Some(Ok(tools)) => Ok((begun.server, tools)),
            Some(Err(problem)) => {
                Err(format!("signed in, but {} still does not answer: {problem}", begun.server))
            }
            None => Err("the server is no longer listed".into()),
        }
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
            // A server that says a great deal about its tools is offered as
            // one Action: said in full on every turn, its tools would cost
            // more than everything else the model is told.
            let as_one = plain(&server.name);
            if weighs_much(tools) && !taken.iter().chain(&specs).any(|other| other.name == as_one) {
                let listed: Vec<String> = tools
                    .iter()
                    .map(|tool| {
                        let about = one_sentence(&tool.description);
                        let about: String = about.chars().take(90).collect();
                        format!("{}: {about}", tool.name)
                    })
                    .collect();
                let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
                specs.push(ActionSpec {
                    name: as_one,
                    description: format!(
                        "The {} tools of {} (an MCP server). `tool` names one and `input` is what it \
                         takes. You do not know what a tool takes until you ask: call with `tool` \
                         alone, nothing is done and its full description and input are returned. \
                         Then call again with `input`.\n{}",
                        tools.len(),
                        server.name,
                        listed.join("\n")
                    ),
                    input: json!({
                        "type": "object",
                        "properties": {
                            "tool": { "type": "string", "enum": names },
                            "input": { "type": "object", "description": "What the tool takes." },
                        },
                        "required": ["tool"],
                    }),
                });
                continue;
            }
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

    /// The server and tool `request` is for, with what the tool is given;
    /// nothing given when the model only asks what the tool takes. The
    /// error is for the model: no such tool on a server offered as one Action.
    fn find(&self, request: &ActionRequest) -> Option<Result<Aimed<'_>, String>> {
        for server in &self.servers {
            let tools = server.tools.lock().unwrap();
            let Some(Ok(tools)) = tools.as_ref() else { continue };
            if let Some(tool) = tools.iter().find(|tool| tool.action == request.name) {
                return Some(Ok((server, tool.clone(), Some(request.input.clone()))));
            }
            if weighs_much(tools) && request.name == plain(&server.name) {
                let wanted = request.input["tool"].as_str().unwrap_or_default();
                let Some(tool) = tools
                    .iter()
                    .find(|tool| tool.name == wanted || plain(&tool.name) == plain(wanted))
                else {
                    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
                    return Some(Err(format!(
                        "{} has no tool called \"{wanted}\". Its tools: {}.",
                        server.name,
                        names.join(", ")
                    )));
                };
                let given = request.input.get("input").filter(|given| !given.is_null()).cloned();
                return Some(Ok((server, tool.clone(), given)));
            }
        }
        None
    }

    /// What the Owner is asked before `request`, when it is for a server's
    /// tool: None inside when the tool only reads.
    pub(crate) fn review(&self, request: &ActionRequest) -> Option<Option<Approval>> {
        // Nothing is asked about what will only be explained to the model.
        let Ok((server, tool, Some(given))) = self.find(request)? else { return Some(None) };
        if tool.only_reads {
            return Some(None);
        }
        let what =
            if tool.title.is_empty() { one_sentence(&tool.description) } else { &tool.title };
        let what = if what.is_empty() { &tool.name } else { what };
        Some(Some(Approval {
            summary: format!("{what} ({})", server.name),
            detail: serde_json::to_string_pretty(&given).unwrap_or_default(),
            irreversible: tool.destroys,
            critical: false,
            contained: false,
        }))
    }

    /// Carries out `request` when it is for a server's tool; the text is its Outcome.
    pub(crate) async fn carry_out(&self, request: &ActionRequest) -> Option<String> {
        let (server, tool, given) = match self.find(request)? {
            Ok(found) => found,
            Err(unknown) => return Some(unknown),
        };
        // Asked what the tool takes: it is said, and nothing is done.
        let Some(given) = given else {
            return Some(format!(
                "{}: {}\nIts input:\n{}\nNothing was done. Call again with `input`.",
                tool.name,
                tool.description,
                serde_json::to_string_pretty(&tool.input).unwrap_or_default()
            ));
        };
        let mut link = server.link.lock().await;
        // What a sign-in obtained does not last: about to end, it is renewed
        // and the server reached afresh with the new one.
        if self
            .signed_in(&server.name)
            .is_some_and(|signed| signed.until.is_some_and(|until| until <= now() + 60))
        {
            *link = None;
        }
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
        let call = json!({ "name": tool.name, "arguments": given });
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

/// True when saying all of `tools` to the model on every turn would cost
/// more than a few thousand tokens.
fn weighs_much(tools: &[Tool]) -> bool {
    let said: usize =
        tools.iter().map(|tool| tool.description.len() + tool.input.to_string().len()).sum();
    said > HEAVY
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

/// A form as programs send one: its fields joined, each written safely.
fn form_written(fields: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new()).extend_pairs(fields).finish()
}
