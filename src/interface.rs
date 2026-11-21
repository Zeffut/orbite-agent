//! The web interface: the Conversation in a browser, served by Orbite
//! itself from the Host. A device is let in by pairing, with a code shown on
//! the Host and used once; there is no password and no account.

use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::{Answer, Approval, Channel, Step};

/// How long a pairing code can be used, in seconds.
const CODE_LASTS: i64 = 600;
/// How many wrong codes are taken before every code is void: nobody finds
/// one by trying them all.
const WRONG_CODES: i64 = 8;
/// How much of the Conversation a device is shown when it opens the page.
const SHOWN_AT_FIRST: i64 = 200;

const PAGE: &str = include_str!("interface/index.html");
const SCRIPT: &str = include_str!("interface/app.js");
const STYLE: &str = include_str!("interface/app.css");
const MANIFEST: &str = include_str!("interface/manifest.webmanifest");
const ICON: &str = include_str!("interface/icon.svg");

/// What a device sent.
enum Incoming {
    /// What was written, or the command a choice stands for when one was
    /// tapped: that one is not shown as something the Owner said.
    Said(String, bool),
    Answered(String, Answer),
}

/// What every connected device is told: the event's number (zero for one
/// that is shown and not kept) and what it is.
type Told = (i64, String);

struct Shared {
    database: Mutex<rusqlite::Connection>,
    to_agent: mpsc::UnboundedSender<Incoming>,
    live: broadcast::Sender<Told>,
    files: PathBuf,
    language: String,
}

/// The Channel of the web interface.
pub struct Interface {
    shared: Arc<Shared>,
    from_devices: mpsc::UnboundedReceiver<Incoming>,
    address: SocketAddr,
    /// The event the Owner's latest words were kept as.
    last_received: Option<i64>,
    asked: u64,
    /// The Approval last asked, as devices know it.
    open_ask: Option<String>,
    serving: tokio::task::JoinHandle<()>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

fn hashed(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unguessable() -> io::Result<String> {
    use base64::Engine;
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random))
}

fn wrong(error: rusqlite::Error) -> io::Error {
    io::Error::other(error)
}

/// What the interface keeps in `home`: the devices let in, the codes that
/// let one in, and what was shown, so that a device finds it again.
fn kept_in(home: &Path) -> io::Result<rusqlite::Connection> {
    std::fs::create_dir_all(home)?;
    let file = home.join("interface.db");
    let database = rusqlite::Connection::open(&file).map_err(wrong)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))?;
    }
    database
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS shown (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL, body TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS devices (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL,
                 token TEXT NOT NULL UNIQUE, since INTEGER NOT NULL, seen INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS codes (
                 code TEXT PRIMARY KEY, until INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS wrong_codes (at INTEGER NOT NULL);",
        )
        .map_err(wrong)?;
    Ok(database)
}

/// A device let in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    /// When it was let in and when it was last seen, in seconds since 1970.
    pub since: u64,
    pub seen: u64,
}

/// A fresh code that lets one device in, once, for a few minutes: shown on
/// the Host, typed on the device. Written as `ABCD-EFGH`.
pub fn pairing_code(home: &Path) -> io::Result<String> {
    // Without the letters and digits that are taken for one another.
    const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let code: String =
        random.iter().map(|byte| LETTERS[usize::from(*byte) % LETTERS.len()] as char).collect();
    let database = kept_in(home)?;
    database.execute("DELETE FROM codes WHERE until < ?1", [now()]).map_err(wrong)?;
    database.execute("DELETE FROM wrong_codes", ()).map_err(wrong)?;
    database
        .execute(
            "INSERT INTO codes (code, until) VALUES (?1, ?2)",
            (hashed(&code), now() + CODE_LASTS),
        )
        .map_err(wrong)?;
    Ok(format!("{}-{}", &code[..4], &code[4..]))
}

/// The devices let in, the latest first.
pub fn devices(home: &Path) -> io::Result<Vec<Device>> {
    let database = kept_in(home)?;
    let mut reading = database
        .prepare("SELECT name, since, seen FROM devices ORDER BY since DESC")
        .map_err(wrong)?;
    let read = reading
        .query_map((), |row| {
            Ok(Device {
                name: row.get(0)?,
                since: row.get::<_, i64>(1)? as u64,
                seen: row.get::<_, i64>(2)? as u64,
            })
        })
        .map_err(wrong)?;
    Ok(read.filter_map(Result::ok).collect())
}

/// Turns away the device called `name`: it is let in no more. True when
/// there was one.
pub fn turn_away(home: &Path, name: &str) -> io::Result<bool> {
    let database = kept_in(home)?;
    Ok(database.execute("DELETE FROM devices WHERE name = ?1", [name]).map_err(wrong)? > 0)
}

/// True when `address` is reached only from the Host itself or from a
/// network of the Owner's own: nothing travels in the open on its way.
pub fn is_private(address: &SocketAddr) -> bool {
    match address.ip() {
        std::net::IpAddr::V4(ip) => {
            let [first, second, ..] = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                // Carrier-grade private space, which Tailscale and its like use.
                || (first == 100 && (64..128).contains(&second))
        }
        std::net::IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            ip.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

impl Interface {
    /// Serves the interface of the installation in `home` at `address`, in
    /// `language` ("en" or "fr"). Port 0 takes any free one.
    pub async fn serving(home: &Path, address: SocketAddr, language: &str) -> io::Result<Self> {
        let database = kept_in(home)?;
        let files = home.join("interface-files");
        std::fs::create_dir_all(&files)?;
        let (to_agent, from_devices) = mpsc::unbounded_channel();
        let (live, _) = broadcast::channel(256);
        let shared = Arc::new(Shared {
            database: Mutex::new(database),
            to_agent,
            live,
            files,
            language: language.to_owned(),
        });
        let listening = tokio::net::TcpListener::bind(address).await?;
        let address = listening.local_addr()?;
        let pages = Router::new()
            .route("/", get(|| async { page("text/html; charset=utf-8", PAGE) }))
            .route("/app.js", get(|| async { page("text/javascript; charset=utf-8", SCRIPT) }))
            .route("/app.css", get(|| async { page("text/css; charset=utf-8", STYLE) }))
            .route("/icon.svg", get(|| async { page("image/svg+xml", ICON) }))
            .route(
                "/manifest.webmanifest",
                get(|| async { page("application/manifest+json", MANIFEST) }),
            )
            .route("/api/pair", post(pair))
            .route("/api/me", get(me))
            .route("/api/events", get(events))
            .route("/api/say", post(say))
            .route("/api/answer", post(answer))
            .route("/api/file", get(file))
            .with_state(shared.clone());
        let serving = tokio::spawn(async move {
            let _ = axum::serve(listening, pages).await;
        });
        Ok(Self {
            shared,
            from_devices,
            address,
            last_received: None,
            asked: 0,
            open_ask: None,
            serving,
        })
    }

    /// Where the interface is served.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Keeps `body` as something shown and tells every connected device.
    fn show(&self, body: Value) -> i64 {
        self.shared.show(body)
    }

    /// Tells every connected device of something that is not kept.
    fn tell(&self, body: Value) {
        let _ = self.shared.live.send((0, body.to_string()));
    }
}

impl Shared {
    fn show(&self, body: Value) -> i64 {
        let written = body.to_string();
        let database = self.database.lock().expect("not poisoned");
        let kept = database
            .execute("INSERT INTO shown (at, body) VALUES (?1, ?2)", (now(), &written))
            .map(|_| database.last_insert_rowid())
            .unwrap_or_default();
        drop(database);
        let _ = self.live.send((kept, written));
        kept
    }

    /// The name of the device that sent `headers`, when it was let in.
    fn device(&self, headers: &HeaderMap) -> Option<String> {
        let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
        let token = cookies
            .split(';')
            .filter_map(|cookie| cookie.trim().strip_prefix("orbite_device="))
            .next()?;
        let database = self.database.lock().expect("not poisoned");
        let name: String = database
            .query_row("SELECT name FROM devices WHERE token = ?1", [hashed(token)], |row| {
                row.get(0)
            })
            .ok()?;
        let _ = database
            .execute("UPDATE devices SET seen = ?1 WHERE token = ?2", (now(), hashed(token)));
        Some(name)
    }
}

impl Drop for Interface {
    fn drop(&mut self) {
        self.serving.abort();
    }
}

fn page(kind: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, kind),
            (header::CACHE_CONTROL, "no-cache"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'self'; img-src 'self' data: blob:; frame-ancestors 'none'; base-uri 'none'",
            ),
        ],
        body,
    )
        .into_response()
}

/// True when a request that changes something comes from the interface's
/// own page: another site a browser has open cannot send it.
fn from_the_page(headers: &HeaderMap) -> bool {
    let own = headers.get("x-orbite").is_some();
    let host = headers.get(header::HOST).and_then(|host| host.to_str().ok());
    let same_place = match headers.get(header::ORIGIN).and_then(|origin| origin.to_str().ok()) {
        None => true,
        Some(origin) => host.is_some_and(|host| {
            origin.strip_prefix("http://").or(origin.strip_prefix("https://")) == Some(host)
        }),
    };
    own && same_place
}

fn refused(status: StatusCode, why: &str) -> Response {
    (status, Json(json!({ "error": why }))).into_response()
}

async fn pair(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    if !from_the_page(&headers) {
        return refused(StatusCode::FORBIDDEN, "not from the interface");
    }
    let code: String = sent["code"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|letter| letter.to_ascii_uppercase())
        .collect();
    let name: String = sent["name"].as_str().unwrap_or_default().trim().chars().take(60).collect();
    let name = if name.is_empty() { "browser".to_owned() } else { name };
    let Ok(token) = unguessable() else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "no token could be made");
    };
    let let_in = {
        let database = shared.database.lock().expect("not poisoned");
        let tried: i64 = database
            .query_row("SELECT COUNT(*) FROM wrong_codes", (), |row| row.get(0))
            .unwrap_or_default();
        let good = tried < WRONG_CODES
            && database
                .execute(
                    "DELETE FROM codes WHERE code = ?1 AND until >= ?2",
                    (hashed(&code), now()),
                )
                .is_ok_and(|used| used == 1);
        if good {
            // A name is a device's own: a second of the same name is numbered.
            let taken: i64 = database
                .query_row(
                    "SELECT COUNT(*) FROM devices WHERE name = ?1 OR name LIKE ?2",
                    (&name, format!("{name} (%")),
                    |row| row.get(0),
                )
                .unwrap_or_default();
            let named = if taken == 0 { name } else { format!("{name} ({})", taken + 1) };
            database
                .execute(
                    "INSERT INTO devices (name, token, since, seen) VALUES (?1, ?2, ?3, ?3)",
                    (named, hashed(&token), now()),
                )
                .is_ok()
        } else {
            let _ = database.execute("INSERT INTO wrong_codes (at) VALUES (?1)", [now()]);
            if tried + 1 >= WRONG_CODES {
                let _ = database.execute("DELETE FROM codes", ());
            }
            false
        }
    };
    if !let_in {
        return refused(StatusCode::UNAUTHORIZED, "wrong code");
    }
    // Sent back by the browser to this place only, and never read by a page.
    let secure = headers
        .get("x-forwarded-proto")
        .and_then(|proto| proto.to_str().ok())
        .is_some_and(|proto| proto == "https");
    let cookie = format!(
        "orbite_device={token}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Strict{}",
        if secure { "; Secure" } else { "" }
    );
    ([(header::SET_COOKIE, cookie)], Json(json!({ "paired": true }))).into_response()
}

async fn me(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    match shared.device(&headers) {
        Some(name) => Json(json!({ "device": name, "language": shared.language })).into_response(),
        None => {
            (StatusCode::UNAUTHORIZED, Json(json!({ "language": shared.language }))).into_response()
        }
    }
}

async fn events(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(asked): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    // Where the device left off: what a page reconnecting by itself says it
    // saw last, or what the page asks for when it opens.
    let seen: Option<i64> = headers
        .get("last-event-id")
        .and_then(|seen| seen.to_str().ok())
        .or(asked.get("after").map(String::as_str))
        .and_then(|seen| seen.parse().ok());
    // Listening starts before what was kept is read: nothing falls between.
    let live = BroadcastStream::new(shared.live.subscribe());
    let earlier: Vec<Told> = {
        let database = shared.database.lock().expect("not poisoned");
        let read = |sql: &str, bound: i64| -> Vec<Told> {
            database
                .prepare(sql)
                .and_then(|mut reading| {
                    reading
                        .query_map([bound], |row| Ok((row.get(0)?, row.get(1)?)))
                        .map(|rows| rows.filter_map(Result::ok).collect())
                })
                .unwrap_or_default()
        };
        match seen {
            Some(seen) => read("SELECT id, body FROM shown WHERE id > ?1 ORDER BY id", seen),
            None => {
                let mut latest =
                    read("SELECT id, body FROM shown ORDER BY id DESC LIMIT ?1", SHOWN_AT_FIRST);
                latest.reverse();
                latest
            }
        }
    };
    let last = earlier.last().map(|(id, _)| *id).or(seen).unwrap_or_default();
    let told = |(id, body): Told| {
        let event = Event::default().data(body);
        Ok::<_, Infallible>(if id > 0 { event.id(id.to_string()) } else { event })
    };
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> = Box::pin(
        tokio_stream::iter(earlier).map(told).chain(
            live.filter_map(move |sent| match sent {
                // What is kept is not told twice to a device that just read it.
                Ok((id, _)) if id > 0 && id <= last => None,
                Ok(sent) => Some(sent),
                // The device fell behind: it reconnects and reads what it missed.
                Err(_) => None,
            })
            .map(told),
        ),
    );
    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}

async fn say(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    if !from_the_page(&headers) || shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let text = sent["text"].as_str().unwrap_or_default().trim().to_owned();
    if text.is_empty() || text.len() > 100_000 {
        return refused(StatusCode::BAD_REQUEST, "nothing to say");
    }
    let tapped = sent["tapped"].as_bool().unwrap_or(false);
    match shared.to_agent.send(Incoming::Said(text, tapped)) {
        Ok(()) => Json(json!({ "sent": true })).into_response(),
        Err(_) => refused(StatusCode::SERVICE_UNAVAILABLE, "the agent is not listening"),
    }
}

async fn answer(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    if !from_the_page(&headers) || shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let answered = match sent["answer"].as_str() {
        Some("allow") => Answer::Allowed,
        Some("always") => Answer::AlwaysAllowed,
        Some("deny") => Answer::Denied,
        _ => return refused(StatusCode::BAD_REQUEST, "allow, always or deny"),
    };
    let asked = sent["ask"].as_str().unwrap_or_default().to_owned();
    match shared.to_agent.send(Incoming::Answered(asked, answered)) {
        Ok(()) => Json(json!({ "sent": true })).into_response(),
        Err(_) => refused(StatusCode::SERVICE_UNAVAILABLE, "the agent is not listening"),
    }
}

async fn file(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(asked): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    // Only what the Agent sent, by the name it was kept under: letters and digits.
    let name = asked.get("id").map(String::as_str).unwrap_or_default();
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        || name.contains("..")
    {
        return refused(StatusCode::NOT_FOUND, "no such file");
    }
    match std::fs::read(shared.files.join(name)) {
        Ok(bytes) => {
            let kind = match name.rsplit('.').next().unwrap_or_default() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                // Anything else is handed over to be saved, never shown as a page.
                _ => "application/octet-stream",
            };
            (
                [
                    (header::CONTENT_TYPE, kind),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                    (header::CACHE_CONTROL, "private, max-age=86400"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => refused(StatusCode::NOT_FOUND, "no such file"),
    }
}

impl Channel for Interface {
    async fn receive(&mut self) -> Option<String> {
        loop {
            match self.from_devices.recv().await? {
                Incoming::Said(text, tapped) => {
                    if !tapped {
                        self.last_received =
                            Some(self.show(json!({ "kind": "owner", "text": text })));
                    }
                    return Some(text);
                }
                // An answer to an Approval that is no longer asked.
                Incoming::Answered(..) => {}
            }
        }
    }

    async fn interruption(&mut self) -> Option<String> {
        self.receive().await
    }

    async fn send(&mut self, text: &str) -> io::Result<()> {
        self.show(json!({ "kind": "agent", "text": text }));
        Ok(())
    }

    async fn offer(&mut self, text: &str, choices: &[(String, String)]) -> io::Result<()> {
        let choices: Vec<Value> = choices
            .iter()
            .map(|(label, command)| json!({ "label": label, "command": command }))
            .collect();
        self.show(json!({ "kind": "agent", "text": text, "choices": choices }));
        Ok(())
    }

    async fn ask(&mut self, approval: &Approval) -> io::Result<Answer> {
        self.asked += 1;
        let asked = format!("{}-{}", now(), self.asked);
        self.open_ask = Some(asked.clone());
        self.show(json!({
            "kind": "ask", "ask": asked, "summary": approval.summary, "detail": approval.detail,
            "irreversible": approval.irreversible, "critical": approval.critical,
        }));
        loop {
            match self.from_devices.recv().await {
                None => return Err(io::Error::other("the interface was closed")),
                Some(Incoming::Answered(to, answer)) if to == asked => {
                    // A Critical Action is never allowed once and for all.
                    let answer = match answer {
                        Answer::AlwaysAllowed if approval.critical => Answer::Allowed,
                        answer => answer,
                    };
                    self.settle(approval, &answer).await;
                    return Ok(answer);
                }
                Some(Incoming::Answered(..)) => {}
                Some(Incoming::Said(text, tapped)) => {
                    if !tapped {
                        self.last_received =
                            Some(self.show(json!({ "kind": "owner", "text": text })));
                    }
                    let answer = Answer::Wrote(text);
                    self.settle(approval, &answer).await;
                    return Ok(answer);
                }
            }
        }
    }

    async fn settle(&mut self, _approval: &Approval, answer: &Answer) {
        let answered = match answer {
            Answer::Allowed => "allow",
            Answer::AlwaysAllowed => "always",
            Answer::Denied => "deny",
            Answer::Wrote(_) => "wrote",
        };
        if let Some(asked) = self.open_ask.take() {
            self.show(json!({ "kind": "settled", "ask": asked, "answer": answered }));
        }
    }

    async fn send_file(&mut self, file: &Path, caption: &str) -> io::Result<()> {
        let ending = file
            .extension()
            .and_then(|ending| ending.to_str())
            .filter(|ending| ending.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("bin")
            .to_lowercase();
        let kept = format!("{}.{ending}", unguessable()?.replace(['-', '_'], "x"));
        std::fs::copy(file, self.shared.files.join(&kept))?;
        let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let image = matches!(ending.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp");
        self.show(json!({
            "kind": "file", "file": kept, "name": name, "caption": caption, "image": image,
        }));
        Ok(())
    }

    async fn echo(&mut self, text: &str) {
        self.show(json!({ "kind": "owner", "text": text, "elsewhere": true }));
    }

    async fn erase_last_received(&mut self) {
        if let Some(event) = self.last_received.take() {
            let _ = self
                .shared
                .database
                .lock()
                .expect("not poisoned")
                .execute("DELETE FROM shown WHERE id = ?1", [event]);
            self.tell(json!({ "kind": "erase", "events": [event] }));
        }
    }

    async fn erase_received_holding(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let holding: Vec<i64> = {
            let database = self.shared.database.lock().expect("not poisoned");
            let found: Vec<(i64, String)> = database
                .prepare("SELECT id, body FROM shown ORDER BY id DESC LIMIT 200")
                .and_then(|mut reading| {
                    reading
                        .query_map((), |row| Ok((row.get(0)?, row.get(1)?)))
                        .map(|rows| rows.filter_map(Result::ok).collect())
                })
                .unwrap_or_default();
            let holding: Vec<i64> = found
                .into_iter()
                .filter(|(_, body)| {
                    serde_json::from_str::<Value>(body).is_ok_and(|shown| {
                        shown["kind"] == "owner"
                            && shown["text"].as_str().is_some_and(|said| said.contains(text))
                    })
                })
                .map(|(id, _)| id)
                .collect();
            for event in &holding {
                let _ = database.execute("DELETE FROM shown WHERE id = ?1", [event]);
            }
            holding
        };
        if !holding.is_empty() {
            self.tell(json!({ "kind": "erase", "events": holding }));
        }
    }

    async fn doing(&mut self, step: &Step) {
        self.tell(json!({ "kind": "step", "icon": step.icon, "text": step.text }));
    }

    async fn idle(&mut self) {
        self.tell(json!({ "kind": "idle" }));
    }

    async fn working(&mut self) {
        self.tell(json!({ "kind": "working" }));
    }
}
