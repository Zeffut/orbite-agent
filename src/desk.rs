//! This computer: the desktop of the machine Orbite runs on, lent to the
//! Agent by the account that opened it. No other account may look at a
//! desktop or act on it, so it is offered from inside: `orbite desk` runs in
//! the Owner's graphical session and takes its orders from the Agent's
//! service, over a socket of the machine on which each side checks which
//! account the other runs as.

use std::io::{self, Read, Write};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::computer::Screen;

/// The name of the socket desktops are offered on. It is no file: nothing
/// is left behind, and who may use it is settled by asking who is there.
pub const SOCKET: &str = "orbite-desk";

/// The largest thing either side sends: a screen of 8K, three bytes a point.
const LARGEST: usize = 7680 * 4320 * 3 + 1024;

fn send(to: &mut UnixStream, said: &[u8]) -> io::Result<()> {
    to.write_all(&(said.len() as u32).to_be_bytes())?;
    to.write_all(said)?;
    to.flush()
}

/// The largest greeting or order: a few words, never a screen.
const WORDS: usize = 64 * 1024;

fn receive(from: &mut UnixStream) -> io::Result<Vec<u8>> {
    receive_at_most(from, LARGEST)
}

fn receive_at_most(from: &mut UnixStream, most: usize) -> io::Result<Vec<u8>> {
    let mut length = [0u8; 4];
    from.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > most {
        return Err(io::Error::other("more was sent than a screen holds"));
    }
    let mut said = vec![0u8; length];
    from.read_exact(&mut said)?;
    Ok(said)
}

/// The account at the other end of `stream`, by its number on this machine.
fn account_of(stream: &UnixStream) -> io::Result<u32> {
    use std::os::fd::AsRawFd;
    let mut who = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: a plain system call, given a place of the size it is told.
    let asked = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(who).cast(),
            &mut size,
        )
    };
    match asked {
        0 => Ok(who.uid),
        _ => Err(io::Error::last_os_error()),
    }
}

/// A desktop offered to the Agent: what it is asked goes to the helper in
/// the session that opened it.
pub struct Offered {
    stream: UnixStream,
    /// The account whose desktop it is, by name.
    pub account: String,
    pub size: (u16, u16),
}

impl Offered {
    fn ask(&mut self, asked: Value) -> io::Result<Value> {
        send(&mut self.stream, asked.to_string().as_bytes())?;
        let answered: Value =
            serde_json::from_slice(&receive(&mut self.stream)?).map_err(io::Error::other)?;
        match answered["ok"] == true {
            true => Ok(answered),
            false => Err(io::Error::other(
                answered["why"].as_str().unwrap_or("the desktop refused").to_owned(),
            )),
        }
    }

    /// What the desktop shows now: its size, then red, green and blue for
    /// each point.
    pub fn points(&mut self) -> io::Result<(u32, u32, Vec<u8>)> {
        let seen = self.ask(json!({ "do": "look" }))?;
        let points = receive(&mut self.stream)?;
        let (width, height) = (
            seen["width"].as_u64().unwrap_or(0) as u32,
            seen["height"].as_u64().unwrap_or(0) as u32,
        );
        if points.len() as u64 != u64::from(width) * u64::from(height) * 3 || width == 0 {
            return Err(io::Error::other("the desktop sent a picture that is not whole"));
        }
        self.size = (width as u16, height as u16);
        Ok((width, height, points))
    }

    pub fn click(&mut self, x: u16, y: u16, twice: bool) -> io::Result<()> {
        self.ask(json!({ "do": "click", "x": x, "y": y, "twice": twice })).map(|_| ())
    }

    pub fn type_text(&mut self, text: &str) -> io::Result<()> {
        self.ask(json!({ "do": "type", "text": text })).map(|_| ())
    }

    pub fn key(&mut self, keys: &str) -> io::Result<()> {
        self.ask(json!({ "do": "key", "keys": keys })).map(|_| ())
    }

    pub fn scroll(&mut self, notches: i32) -> io::Result<()> {
        self.ask(json!({ "do": "scroll", "notches": notches })).map(|_| ())
    }

    /// Opens `address` with the session's own browser.
    pub fn open(&mut self, address: &str) -> io::Result<()> {
        self.ask(json!({ "do": "open", "address": address })).map(|_| ())
    }
}

/// Where the desktop on offer is kept, for whoever acts on it.
pub type Lent = Arc<Mutex<Option<Offered>>>;

/// Waits on the socket called `name` for the desktop of the account numbered
/// `trusted`, and keeps the latest one offered in `lent`. Any other account
/// is told no and nothing of it is kept. Returns once listening, or says
/// why it could not (another Orbite of this machine is listening already).
pub fn listen(name: &str, trusted: u32, lent: Lent) -> io::Result<()> {
    let address = SocketAddr::from_abstract_name(name)?;
    let first = UnixListener::bind_addr(&address);
    let failed = first.as_ref().err().map(|error| io::Error::new(error.kind(), error.to_string()));
    std::thread::spawn(move || {
        // A name taken by something else is asked for again until it is
        // free: whoever took it is not obeyed by any helper meanwhile.
        let listening = match first {
            Ok(listening) => listening,
            Err(_) => loop {
                std::thread::sleep(Duration::from_secs(10));
                if let Ok(listening) = UnixListener::bind_addr(&address) {
                    break listening;
                }
            },
        };
        for offering in listening.incoming() {
            let Ok(offering) = offering else { continue };
            // Who it is comes first, before a word of theirs is read; and
            // each is heard apart, so that none keeps the others waiting.
            let known = account_of(&offering).is_ok_and(|account| account == trusted);
            let lent = lent.clone();
            std::thread::spawn(move || welcome(offering, known, &lent));
        }
    });
    match failed {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Hears one caller out: the account whose desktop Orbite takes is lent
/// from, anyone else is told no.
fn welcome(mut offering: UnixStream, known: bool, lent: &Lent) {
    let _ = offering.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = offering.set_write_timeout(Some(Duration::from_secs(20)));
    // Heard out before being answered: an answer sent before the other
    // side has spoken is one it may never read. A few words at most.
    let said = receive_at_most(&mut offering, WORDS)
        .ok()
        .and_then(|said| serde_json::from_slice::<Value>(&said).ok());
    let (Some(said), true) = (said, known) else {
        let refused = json!({ "ok": false, "why": "not the account Orbite was installed by" });
        let _ = send(&mut offering, refused.to_string().as_bytes());
        return;
    };
    if said["session"] != "x11" {
        // Told plainly, and nothing is lent: the Agent keeps a computer of its own.
        let _ =
            send(&mut offering, json!({ "ok": false, "why": "wayland" }).to_string().as_bytes());
        return;
    }
    if send(&mut offering, json!({ "ok": true }).to_string().as_bytes()).is_err() {
        return;
    }
    let _ = offering.set_read_timeout(Some(Duration::from_secs(20)));
    let size =
        (said["width"].as_u64().unwrap_or(0) as u16, said["height"].as_u64().unwrap_or(0) as u16);
    // A name as accounts are named: nothing else of it reaches a model.
    let account: String = said["account"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(32)
        .collect();
    *lent.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some(Offered { stream: offering, account, size });
}

/// Listens as [`listen`] does and gives a way to ask whether a desktop is
/// lent now.
pub fn lending(name: &str, trusted: u32) -> io::Result<impl Fn() -> bool> {
    let lent = Lent::default();
    listen(name, trusted, lent.clone())?;
    Ok(move || lent.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_some())
}

/// Why a desktop could not be offered, as said to whoever ran `orbite desk`.
#[derive(Debug)]
pub enum NotOffered {
    /// No Orbite is listening on this machine.
    NobodyListening,
    /// What listens is not the Agent's own service.
    NotOrbite,
    /// This account is not the one whose desktop Orbite takes.
    NotTheOwner,
    /// The session is of a kind whose desktop cannot be lent yet.
    Wayland,
    Other(String),
}

/// Offers the desktop `screen` shows, in the name of `account`, to the
/// Orbite of this machine whose service runs as the account numbered
/// `agent`, and does what it asks for as long as it listens. `session` is
/// the kind of the graphical session, "x11" or "wayland".
pub fn offer(
    name: &str,
    agent: u32,
    account: &str,
    session: &str,
    screen: &Screen,
) -> Result<(), NotOffered> {
    let other = |error: io::Error| NotOffered::Other(error.to_string());
    let address = SocketAddr::from_abstract_name(name).map_err(other)?;
    let mut service =
        UnixStream::connect_addr(&address).map_err(|_| NotOffered::NobodyListening)?;
    if account_of(&service).map_err(other)? != agent {
        return Err(NotOffered::NotOrbite);
    }
    let (width, height) = match session {
        "x11" => screen.points().map(|(width, height, _)| (width, height)).map_err(other)?,
        _ => (0, 0),
    };
    let hello = json!({ "account": account, "session": session, "width": width, "height": height });
    send(&mut service, hello.to_string().as_bytes()).map_err(other)?;
    let welcomed: Value = serde_json::from_slice(&receive(&mut service).map_err(other)?)
        .map_err(|e| other(e.into()))?;
    if welcomed["ok"] != true {
        return Err(match welcomed["why"].as_str() {
            Some("wayland") => NotOffered::Wayland,
            _ => NotOffered::NotTheOwner,
        });
    }
    loop {
        let asked: Value = match receive_at_most(&mut service, WORDS) {
            Ok(asked) => serde_json::from_slice(&asked).unwrap_or_default(),
            // The service stopped: the offer ends with it.
            Err(_) => return Ok(()),
        };
        let text = |key: &str| asked[key].as_str().unwrap_or_default();
        let number = |key: &str| asked[key].as_i64().unwrap_or(0);
        let done: io::Result<Option<(u32, u32, Vec<u8>)>> = match text("do") {
            "look" => screen.points().map(Some),
            "click" if asked["twice"] == true => {
                screen.double_click(number("x") as u16, number("y") as u16).map(|()| None)
            }
            "click" => screen.click(number("x") as u16, number("y") as u16).map(|()| None),
            "type" => screen.type_text(text("text")).map(|()| None),
            "key" => screen.key(text("keys")).map(|()| None),
            "scroll" => screen.scroll(number("notches").clamp(-50, 50) as i32).map(|()| None),
            "open" => open_in_the_session(text("address")).map(|()| None),
            _ => Err(io::Error::other("not something a desktop is asked")),
        };
        let sent = match done {
            Ok(Some((width, height, points))) => send(
                &mut service,
                json!({ "ok": true, "width": width, "height": height }).to_string().as_bytes(),
            )
            .and_then(|()| send(&mut service, &points)),
            Ok(None) => send(&mut service, json!({ "ok": true }).to_string().as_bytes()),
            Err(why) => send(
                &mut service,
                json!({ "ok": false, "why": why.to_string() }).to_string().as_bytes(),
            ),
        };
        if sent.is_err() {
            return Ok(());
        }
    }
}

/// Opens a page of the web with the session's own browser, and nothing else.
fn open_in_the_session(address: &str) -> io::Result<()> {
    if !(address.starts_with("https://") || address.starts_with("http://")) {
        return Err(io::Error::other("only a page of the web is opened"));
    }
    std::process::Command::new("xdg-open")
        .arg(address)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}
