//! The web interface: the Conversation in a browser, served by Orbite
//! itself from the Host. A device is let in by pairing, with a code shown on
//! the Host and used once; there is no password and no account.

use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{DefaultBodyLimit, Query, State};
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
/// The largest file a device may send, in bytes.
const UPLOAD_LIMIT: usize = 50_000_000;
/// How many files may wait to be sent with a message.
const WAITING_FILES: usize = 20;
/// How much of the Conversation a device is shown when it opens the page.
const SHOWN_AT_FIRST: i64 = 200;

const PAGE: &str = include_str!("interface/index.html");
const SCRIPT: &str = include_str!("interface/app.js");
const STYLE: &str = include_str!("interface/app.css");
const MANIFEST: &str = include_str!("interface/manifest.webmanifest");
const ICON: &str = include_str!("interface/icon.svg");
/// The same icon as pictures, for the phones that take nothing else when
/// the page is put on their home screen.
const ICONS: [(&str, &[u8]); 4] = [
    ("/icon-192.png", include_bytes!("interface/icon-192.png")),
    ("/icon-512.png", include_bytes!("interface/icon-512.png")),
    ("/icon-maskable.png", include_bytes!("interface/icon-maskable.png")),
    ("/icon-touch.png", include_bytes!("interface/icon-touch.png")),
];

/// What a device sent.
enum Incoming {
    /// What was written, and how it is shown as something the Owner said;
    /// the command a choice stands for when one was tapped is not shown.
    Said(String, Option<Value>),
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
    /// Where what the Owner sends is saved for the Agent.
    inbox: PathBuf,
    /// The installation's folder, where its settings are read.
    home: PathBuf,
    /// Where the Owner's devices reach the interface, when it is encrypted
    /// there: a sign-in to a service can end on it.
    reached_at: Option<String>,
    /// The services connected with a touch.
    connectors: Vec<crate::Connector>,
    /// The sign-in to a service that is under way, begun from a device.
    signing_in: tokio::sync::Mutex<Option<crate::mcp::Mcp>>,
    /// Files received and not yet sent with a message, oldest first.
    waiting: Mutex<Vec<Waiting>>,
    /// The Agent's screen as last seen, and how many times it was: kept in
    /// memory alone.
    screen: Mutex<(u64, Vec<u8>)>,
    /// The names the interface is served under, beside its address.
    names: Vec<String>,
    /// True when the Owner's devices reach the interface encrypted.
    encrypted: bool,
    /// What the Owner called their Agent.
    agent: String,
    /// How the Agent looks, when its Owner chose.
    look: Option<crate::Look>,
    language: String,
}

/// A file a device sent, waiting for the message it goes with.
struct Waiting {
    /// What the device names it by.
    token: String,
    /// Where it is saved for the Agent.
    saved: PathBuf,
    /// The name it is shown and fetched under.
    kept: String,
    name: String,
}

impl Waiting {
    /// Removes the file, which no message took.
    fn dropped(&self, files: &Path) {
        let _ = std::fs::remove_file(&self.saved);
        let _ = std::fs::remove_file(files.join(&self.kept));
    }
}

/// True for what a page may show as a picture.
fn is_a_picture(ending: &str) -> bool {
    matches!(ending, "png" | "jpg" | "jpeg" | "gif" | "webp")
}

/// The ending of `file`'s name, in small letters: "bin" when it has none
/// that can be trusted in a name.
fn ending_of(file: &Path) -> String {
    file.extension()
        .and_then(|ending| ending.to_str())
        .filter(|ending| !ending.is_empty() && ending.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin")
        .to_lowercase()
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

/// Keeps `text` as the Agent's first words in the interface of `home`,
/// when nothing was said there yet: the first device let in is greeted.
pub fn greet(home: &Path, text: &str) -> io::Result<()> {
    let database = kept_in(home)?;
    let said: i64 =
        database.query_row("SELECT COUNT(*) FROM shown", [], |row| row.get(0)).map_err(wrong)?;
    if said == 0 {
        let body = json!({ "kind": "agent", "text": text }).to_string();
        database
            .execute("INSERT INTO shown (at, body) VALUES (?1, ?2)", (now(), body))
            .map_err(wrong)?;
    }
    Ok(())
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
        Self::serving_with(home, address, language, crate::CONNECTORS).await
    }

    /// The same, offering `connectors` as the services connected with a touch.
    pub async fn serving_with(
        home: &Path,
        address: SocketAddr,
        language: &str,
        offered: &[crate::Connector],
    ) -> io::Result<Self> {
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
            inbox: home.join("workspace/inbox"),
            home: home.to_owned(),
            reached_at: crate::Config::load(home)
                .ok()
                .and_then(|config| config.interface_reached_at)
                .filter(|reached| reached.starts_with("https://")),
            connectors: offered.to_vec(),
            signing_in: tokio::sync::Mutex::new(None),
            waiting: Mutex::new(Vec::new()),
            screen: Mutex::new((0, Vec::new())),
            encrypted: crate::Config::load(home)
                .ok()
                .and_then(|config| config.interface_reached_at)
                .is_some_and(|reached| reached.starts_with("https://")),
            names: crate::Config::load(home)
                .ok()
                .and_then(|config| config.interface_reached_at)
                .and_then(|reached| {
                    let named = reached.split("://").nth(1)?.split(['/', ':']).next()?.to_owned();
                    Some(vec![named])
                })
                .unwrap_or_default(),
            agent: crate::Config::load(home)
                .ok()
                .and_then(|config| config.agent_name)
                .unwrap_or_else(|| "Orbite".to_owned()),
            look: crate::Config::load(home).ok().filter(|config| config.agent_shape.is_some()).map(
                |config| {
                    crate::Look::called(
                        config.agent_shape.as_deref(),
                        config.agent_colour.as_deref(),
                    )
                },
            ),
            language: language.to_owned(),
        });
        let listening = tokio::net::TcpListener::bind(address).await?;
        let address = listening.local_addr()?;
        let mut pages = Router::new();
        for (path, picture) in ICONS {
            pages = pages.route(
                path,
                get(move || async move {
                    (
                        [
                            (header::CONTENT_TYPE, "image/png"),
                            (header::CACHE_CONTROL, "public, max-age=86400"),
                        ],
                        picture,
                    )
                }),
            );
        }
        let pages = pages
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
            .route("/api/settings", get(settings).post(set))
            .route("/api/welcomed", post(welcomed))
            .route("/api/connectors", get(connectors).post(connect))
            .route("/signed-in", get(signed_in))
            .route("/api/file", get(file))
            .route("/api/screen", get(screen))
            .route(
                "/api/upload",
                post(upload).delete(give_up).layer(DefaultBodyLimit::max(UPLOAD_LIMIT)),
            )
            .layer(axum::middleware::from_fn_with_state(shared.clone(), asked_by_its_name))
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

    /// Shows what the Owner just said, as the device that sent it had it.
    fn received(&mut self, shown: Option<Value>) {
        if let Some(shown) = shown {
            self.last_received = Some(self.show(shown));
        }
    }

    /// Tells every connected device of something that is not kept.
    fn tell(&self, body: Value) {
        let _ = self.shared.live.send((0, body.to_string()));
    }
}

impl Shared {
    fn show(&self, mut body: Value) -> i64 {
        // When it was said, for the page to mark the hours of the Conversation.
        body["at"] = json!(now());
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

impl Shared {
    /// True while the device that sent `headers` is let in.
    fn is_let_in(&self, headers: &HeaderMap) -> bool {
        let token = headers.get(header::COOKIE).and_then(|cookies| cookies.to_str().ok()).and_then(
            |cookies| {
                cookies
                    .split(';')
                    .filter_map(|cookie| cookie.trim().strip_prefix("orbite_device="))
                    .next()
            },
        );
        let Some(token) = token else { return false };
        self.database
            .lock()
            .expect("not poisoned")
            .query_row("SELECT 1 FROM devices WHERE token = ?1", [hashed(token)], |_| Ok(()))
            .is_ok()
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

/// Answers only what was asked of the interface by a name of its own: the
/// address it listens on, the Host itself, or the name it is served under.
/// A page of another site that has a browser reach this place under its
/// own name is refused, whatever it then asks.
async fn asked_by_its_name(
    State(shared): State<Arc<Shared>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let asked = request.headers().get(header::HOST).and_then(|host| host.to_str().ok());
    let name = asked.map(|host| match host.rsplit_once(':') {
        // Not the colons of an address written in brackets.
        Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    });
    let own = name.is_some_and(|name| {
        name.eq_ignore_ascii_case("localhost")
            || name.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok()
            || shared.names.iter().any(|own| own.eq_ignore_ascii_case(name))
    });
    if !own {
        return refused(StatusCode::MISDIRECTED_REQUEST, "not this place");
    }
    next.run(request).await
}

/// True when a request that changes something comes from the interface's
/// own page: another site a browser has open cannot send it.
fn from_the_page(headers: &HeaderMap, names: &[String]) -> bool {
    let own = headers.get("x-orbite").is_some();
    // The place the browser asked for: as it reached Orbite, or one of the
    // names Orbite is served under encrypted, which no request can make up.
    let host = headers.get(header::HOST).and_then(|host| host.to_str().ok());
    let same_place = match headers.get(header::ORIGIN).and_then(|origin| origin.to_str().ok()) {
        None => true,
        Some(origin) => {
            let from = origin.strip_prefix("http://").or(origin.strip_prefix("https://"));
            let name = from.map(|from| from.rsplit_once(':').map_or(from, |(name, _)| name));
            from.is_some()
                && (from == host
                    || name
                        .is_some_and(|name| names.iter().any(|own| own.eq_ignore_ascii_case(name))))
        }
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
    if !from_the_page(&headers, &shared.names) {
        return refused(StatusCode::FORBIDDEN, "not from the interface");
    }
    let code: String = sent["code"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|letter| letter.to_ascii_uppercase())
        .collect();
    // A name the Owner can read and type: letters, digits and a few marks.
    let name: String = sent["name"]
        .as_str()
        .unwrap_or_default()
        .chars()
        // Latin letters, accented ones too: none that shows as nothing, or
        // as a letter it is not.
        .filter(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, ' ' | '·' | '-' | '_' | '.' | '\'')
                || (('\u{c0}'..='\u{24f}').contains(c) && c.is_alphabetic())
        })
        .take(60)
        .collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = if name.is_empty() { "browser".to_owned() } else { name };
    let Ok(token) = unguessable() else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "no token could be made");
    };
    let let_in = {
        let database = shared.database.lock().expect("not poisoned");
        // The right code is let in whatever was tried before it: a
        // stranger who guesses at random must not be able to keep the
        // Owner out. What stops guessing is how many codes there are.
        let good = database
            .execute("DELETE FROM codes WHERE code = ?1 AND until >= ?2", (hashed(&code), now()))
            .is_ok_and(|used| used == 1);
        if good {
            // A name is a device's own: a second of the same name is numbered.
            let taken = |name: &str| {
                database
                    .query_row("SELECT 1 FROM devices WHERE name = ?1", [name], |_| Ok(()))
                    .is_ok()
            };
            let named = (1..)
                .map(|nth| if nth == 1 { name.clone() } else { format!("{name} {nth}") })
                .find(|named| !taken(named))
                .unwrap_or(name);
            database
                .execute(
                    "INSERT INTO devices (name, token, since, seen) VALUES (?1, ?2, ?3, ?3)",
                    (named, hashed(&token), now()),
                )
                .is_ok()
        } else {
            false
        }
    };
    if !let_in {
        // A wrong code is answered slowly: of the million million there
        // are, few can be tried in the ten minutes one lasts.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        return refused(StatusCode::UNAUTHORIZED, "wrong code");
    }
    // Sent back by the browser to this place only, and never read by a page.
    let secure = shared.encrypted
        || headers
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
        Some(name) => {
            let seen = shared.screen.lock().expect("not poisoned").0;
            Json(json!({
                "device": name, "language": shared.language, "screen": seen,
                "agent": shared.agent,
                // True once the Owner has been shown round, on any device.
                "welcomed": crate::Store::open(shared.home.join("orbite.db"))
                    .ok()
                    .and_then(|store| store.setting("welcomed").ok().flatten())
                    .is_some(),
                // Drawn by the page: the outline, the colour, where the eyes sit.
                "look": shared.look.map(|look| json!({
                    "shape": look.shape, "colour": look.on_a_page(),
                    "outline": look.outline(), "eyes": 36.0 + 42.0 * look.eyes_down(),
                })),
            }))
            .into_response()
        }
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
    // A device turned away is told nothing more, even on a page left open.
    let (still, kept) = (shared.clone(), headers.clone());
    let live =
        BroadcastStream::new(shared.live.subscribe()).take_while(move |_| still.is_let_in(&kept));
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
        tokio_stream::iter(earlier)
            // Then a word that this is all there was: what follows is new.
            .chain(tokio_stream::iter([(0, json!({ "kind": "caught-up" }).to_string())]))
            .map(told)
            .chain(
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
    if !from_the_page(&headers, &shared.names) || shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let mut text = sent["text"].as_str().unwrap_or_default().trim().to_owned();
    // The files sent ahead to go with these words.
    let named: Vec<&str> =
        sent["files"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let with: Vec<Waiting> = {
        let mut waiting = shared.waiting.lock().expect("not poisoned");
        if named.iter().any(|token| !waiting.iter().any(|one| one.token == *token)) {
            return refused(StatusCode::GONE, "a file is no longer there: send it again");
        }
        let (with, left) =
            waiting.drain(..).partition(|one: &Waiting| named.contains(&one.token.as_str()));
        *waiting = left;
        with
    };
    if (text.is_empty() && with.is_empty()) || text.len() > 100_000 {
        return refused(StatusCode::BAD_REQUEST, "nothing to say");
    }
    let tapped = sent["tapped"].as_bool().unwrap_or(false) && with.is_empty();
    let shown = (!tapped).then(|| {
        let files: Vec<Value> = with
            .iter()
            .map(|one| {
                json!({
                    "file": one.kept, "name": one.name,
                    "image": is_a_picture(&ending_of(Path::new(&one.kept))),
                })
            })
            .collect();
        match files.is_empty() {
            true => json!({ "kind": "owner", "text": text }),
            false => json!({ "kind": "owner", "text": text, "files": files }),
        }
    });
    for one in &with {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&crate::attached(&one.saved));
    }
    match shared.to_agent.send(Incoming::Said(text, shown)) {
        Ok(()) => Json(json!({ "sent": true })).into_response(),
        Err(_) => refused(StatusCode::SERVICE_UNAVAILABLE, "the agent is not listening"),
    }
}

async fn answer(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    if !from_the_page(&headers, &shared.names) || shared.device(&headers).is_none() {
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

/// The services connected with a touch, and where each stands: connected,
/// or only waiting to be.
async fn connectors(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let french = shared.language == "fr";
    let all: Vec<Value> = shared
        .connectors
        .iter()
        .map(|connector| {
            json!({
                "name": connector.name,
                "label": connector.label,
                "does": if french { connector.french } else { connector.english },
                "connected": crate::mcp::Mcp::is_kept_signed_in_to(&shared.home, connector.name),
            })
        })
        .collect();
    // With a touch only where a sign-in can end on the Agent's own page.
    Json(json!({ "connectors": all, "with_a_touch": shared.reached_at.is_some() })).into_response()
}

/// Begins connecting a service: it is added to those the Agent knows, and
/// the link to sign in with is given back.
async fn connect(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    if !from_the_page(&headers, &shared.names) || shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let Some(connector) =
        shared.connectors.iter().find(|connector| Some(connector.name) == sent["name"].as_str())
    else {
        return refused(StatusCode::BAD_REQUEST, "not a service known here");
    };
    let Some(reached_at) = &shared.reached_at else {
        return refused(
            StatusCode::CONFLICT,
            "the agent's page is not reached encrypted: ask the agent to connect it instead",
        );
    };
    if !crate::mcp::listed(&shared.home).iter().any(|listed| listed == connector.name)
        && let Err(problem) =
            crate::mcp::add_server(&shared.home, connector.name, connector.address)
    {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, &problem);
    }
    let signing = crate::mcp::Mcp::configured_in(&shared.home)
        .ending_sign_ins_at(format!("{reached_at}/signed-in"));
    match signing.sign_in(connector.name).await {
        Ok(link) => {
            *shared.signing_in.lock().await = Some(signing);
            Json(json!({ "link": link })).into_response()
        }
        Err(problem) => refused(StatusCode::BAD_GATEWAY, &problem),
    }
}

/// Where a sign-in to a service ends: the Owner's browser comes back here
/// with what the service gave. Only the answer to the link that was sent
/// is taken; anything else changes nothing and reaches nobody.
async fn signed_in(
    State(shared): State<Arc<Shared>>,
    axum::extract::RawQuery(asked): axum::extract::RawQuery,
) -> Response {
    let french = shared.language == "fr";
    let page = |well: bool, said: &str| {
        let mark = if well { "✓" } else { "✗" };
        let back = if french { "Tu peux revenir à Orbite." } else { "You can go back to Orbite." };
        axum::response::Html(format!(
            "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, \
             initial-scale=1\"><title>Orbite</title><body style=\"margin:0;min-height:100vh;display:grid;\
             place-items:center;background:#0d0d0d;color:#ededed;font:17px/1.5 system-ui,sans-serif;\
             text-align:center\"><div><div style=\"font-size:54px\">{mark}</div><p><strong>{}</strong></p>\
             <p style=\"color:#8b8b8b\">{back}</p></div>",
            html_text(said)
        ))
        .into_response()
    };
    let Some(signing) = shared.signing_in.lock().await.take() else {
        return page(
            false,
            if french { "Aucune connexion en cours." } else { "No sign-in is under way." },
        );
    };
    let Some(reached_at) = &shared.reached_at else {
        return page(false, "No sign-in is under way.");
    };
    let ended_on = format!("{reached_at}/signed-in?{}", asked.unwrap_or_default());
    match signing.signed_in_at(&ended_on).await {
        Ok((name, _)) => {
            // The Agent starts again to take the service it now has.
            let _ = shared.to_agent.send(Incoming::Said("/reload".into(), None));
            let label = shared
                .connectors
                .iter()
                .find(|connector| connector.name == name)
                .map_or(name.as_str(), |connector| connector.label)
                .to_owned();
            page(
                true,
                &format!("{label} {}", if french { "est connecté." } else { "is connected." }),
            )
        }
        Err(_) => page(
            false,
            if french { "La connexion n'a pas abouti." } else { "The sign-in did not go through." },
        ),
    }
}

/// `text` as it may stand in a page, whatever it holds.
fn html_text(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The Owner was shown round their Agent, on the device that says so: it
/// is not shown again, there or on any other.
async fn welcomed(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !from_the_page(&headers, &shared.names) || shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let kept = crate::Store::open(shared.home.join("orbite.db"))
        .and_then(|store| store.set_setting("welcomed", Some("on")));
    match kept {
        Ok(()) => Json(json!({ "welcomed": true })).into_response(),
        Err(_) => refused(StatusCode::SERVICE_UNAVAILABLE, "it could not be kept"),
    }
}

/// The settings, for a device to show as an app shows its own: each with
/// what it is set to and what it can be set to.
async fn settings(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let Ok(store) = crate::Store::open(shared.home.join("orbite.db")) else {
        return refused(StatusCode::SERVICE_UNAVAILABLE, "the settings cannot be read");
    };
    let read = crate::Settings::of(&store, Some(&shared.home)).waking_up_by_default_every(240);
    let all: Vec<Value> = crate::SETTINGS
        .iter()
        .map(|key| {
            json!({
                "key": key,
                "value": read.get(key).unwrap_or_default(),
                "choices": crate::Settings::choices(key),
            })
        })
        .collect();
    // What each shape and colour looks like, for a device to draw them.
    let shapes: serde_json::Map<String, Value> = crate::SHAPES
        .iter()
        .map(|shape| {
            let look = crate::Look::called(Some(shape), None);
            let drawn =
                json!({ "outline": look.outline(), "eyes": 36.0 + 42.0 * look.eyes_down() });
            ((*shape).to_owned(), drawn)
        })
        .collect();
    let colours: serde_json::Map<String, Value> =
        crate::COLOURS.iter().map(|(name, page, _)| ((*name).to_owned(), json!(page))).collect();
    Json(json!({ "settings": all, "shapes": shapes, "colours": colours })).into_response()
}

/// A setting changed from a device: handed to the Agent, which changes it
/// as it would had its Owner asked in words, and says nothing of it.
async fn set(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(sent): Json<Value>,
) -> Response {
    if !from_the_page(&headers, &shared.names) || shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let (Some(key), Some(value)) = (sent["key"].as_str(), sent["value"].as_str()) else {
        return refused(StatusCode::BAD_REQUEST, "a key and a value");
    };
    let one_line = |text: &str| !text.is_empty() && !text.contains(['\n', '\r']);
    if !crate::SETTINGS.contains(&key) || !one_line(value) || value.chars().count() > 200 {
        return refused(StatusCode::BAD_REQUEST, "not a setting, or not a value for one");
    }
    match shared.to_agent.send(Incoming::Said(format!("/set {key} {value}"), None)) {
        Ok(()) => Json(json!({ "sent": true })).into_response(),
        Err(_) => refused(StatusCode::SERVICE_UNAVAILABLE, "the agent is not listening"),
    }
}

/// The Agent's screen as it was last seen.
async fn screen(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let picture = shared.screen.lock().expect("not poisoned").1.clone();
    if picture.is_empty() {
        return refused(StatusCode::NOT_FOUND, "no screen yet");
    }
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        picture,
    )
        .into_response()
}

/// Takes in a file to go with the next message: saved where the Agent
/// reads what its Owner sends, and named by something only this device is
/// told.
async fn upload(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(asked): Query<std::collections::HashMap<String, String>>,
    content: axum::body::Body,
) -> Response {
    // Nothing of what is sent is read before the device is known.
    if !from_the_page(&headers, &shared.names) || shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let Ok(content) = axum::body::to_bytes(content, UPLOAD_LIMIT).await else {
        return refused(StatusCode::PAYLOAD_TOO_LARGE, "too big");
    };
    if content.is_empty() {
        return refused(StatusCode::BAD_REQUEST, "an empty file");
    }
    // A name as people write them: no way out of the inbox, nothing unseen.
    let name: String = asked
        .get("name")
        .map(String::as_str)
        .unwrap_or("file")
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | '[' | ']'))
        .take(120)
        .collect();
    let name = match name.trim().trim_start_matches('.') {
        "" => "file".to_owned(),
        name => name.to_owned(),
    };
    let taken = || -> io::Result<Waiting> {
        let saved = crate::attachments::save_in(&shared.inbox, &name, &content)?;
        let kept = format!("{}.{}", unguessable()?.replace(['-', '_'], "x"), ending_of(&saved));
        // The same file under both names where that can be, a copy otherwise.
        let shown = shared.files.join(&kept);
        if std::fs::hard_link(&saved, &shown).is_err() {
            std::fs::copy(&saved, &shown)?;
        }
        Ok(Waiting { token: unguessable()?, saved, kept, name: name.clone() })
    };
    match taken() {
        Ok(one) => {
            let token = one.token.clone();
            let mut waiting = shared.waiting.lock().expect("not poisoned");
            waiting.push(one);
            // Never more than a few: the oldest that no message took goes.
            while waiting.len() > WAITING_FILES {
                waiting.remove(0).dropped(&shared.files);
            }
            Json(json!({ "file": token })).into_response()
        }
        Err(error) => refused(StatusCode::INSUFFICIENT_STORAGE, &error.to_string()),
    }
}

/// Forgets a file sent ahead that its Owner took back before sending.
async fn give_up(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(asked): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if !from_the_page(&headers, &shared.names) || shared.device(&headers).is_none() {
        return refused(StatusCode::UNAUTHORIZED, "not paired");
    }
    let token = asked.get("file").map(String::as_str).unwrap_or_default();
    let mut waiting = shared.waiting.lock().expect("not poisoned");
    if let Some(at) = waiting.iter().position(|one| one.token == token) {
        waiting.remove(at).dropped(&shared.files);
    }
    Json(json!({ "gone": true })).into_response()
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
                Incoming::Said(text, shown) => {
                    self.received(shown);
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

    async fn send_unkept(&mut self, text: &str) -> io::Result<()> {
        // Told to the devices open now, and written nowhere. A link that
        // lets a device in is drawn too, as a code a phone's camera reads.
        let link = text
            .split_whitespace()
            .find(|word| word.starts_with("http") && word.contains("/#"))
            .filter(|link| {
                !["http://127.", "http://localhost"].iter().any(|own| link.starts_with(own))
            });
        let code = link.and_then(crate::code_picture);
        self.tell(json!({ "kind": "agent", "text": text, "code": code }));
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
                Some(Incoming::Said(text, shown)) => {
                    self.received(shown);
                    let answer = Answer::Wrote(text);
                    self.settle(approval, &answer).await;
                    return Ok(answer);
                }
            }
        }
    }

    async fn written_before(&mut self) -> Vec<String> {
        // What devices sent that was not read yet: written before whatever
        // Orbite is about to ask.
        let mut written = Vec::new();
        while let Ok(sent) = self.from_devices.try_recv() {
            if let Incoming::Said(text, shown) = sent {
                self.received(shown);
                written.push(text);
            }
        }
        written
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
        let ending = ending_of(file);
        let kept = format!("{}.{ending}", unguessable()?.replace(['-', '_'], "x"));
        std::fs::copy(file, self.shared.files.join(&kept))?;
        let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let image = is_a_picture(&ending);
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
                            && shown["text"]
                                .as_str()
                                .is_some_and(|said| crate::secrets::holds_whole(said, text))
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

    async fn screen(&mut self, picture: &Path) {
        let Ok(seen) = std::fs::read(picture) else { return };
        let times = {
            let mut screen = self.shared.screen.lock().expect("not poisoned");
            *screen = (screen.0 + 1, seen);
            screen.0
        };
        self.tell(json!({ "kind": "screen", "seen": times }));
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
