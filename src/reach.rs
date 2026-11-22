//! How the web interface is reached from the Owner's other devices without
//! anything travelling in the open: Orbite listens on the Host alone, and
//! Tailscale, where the Host has it, serves that over HTTPS inside the
//! Owner's own network, with a certificate it gets and renews itself.

use std::net::SocketAddr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// What `tailscale` answered to `asked`, or why it did not: it is given a
/// few seconds, since it may wait for someone to act where nobody is.
fn asked_of(tailscale: &Path, asked: &[&str]) -> Result<String, String> {
    let start = || {
        Command::new(tailscale)
            .args(asked)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    };
    // A program still being written, as one just updated may be, is tried
    // again a moment later.
    let mut tries = 0;
    let mut running = loop {
        match start() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            started => break started.map_err(|error| error.to_string())?,
        }
    };
    let started = Instant::now();
    loop {
        match running.try_wait().map_err(|error| error.to_string())? {
            Some(_) => break,
            None if started.elapsed() > Duration::from_secs(20) => {
                let _ = running.kill();
                let _ = running.wait();
                return Err("Tailscale did not answer".into());
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let ended = running.wait_with_output().map_err(|error| error.to_string())?;
    if ended.status.success() {
        return Ok(String::from_utf8_lossy(&ended.stdout).into_owned());
    }
    let said = String::from_utf8_lossy(&ended.stderr);
    let why = said.lines().find(|line| !line.trim().is_empty()).unwrap_or("Tailscale refused");
    Err(why.trim().to_owned())
}

/// The `tailscale` program of this machine, when it has one.
pub fn tailscale() -> Option<std::path::PathBuf> {
    // As root, only the one the system keeps among its own programs:
    // whoever had Orbite run as root has no say in which program that is.
    // SAFETY: a plain system call.
    #[cfg(unix)]
    if unsafe { libc::geteuid() } == 0 {
        let system = crate::system_program("tailscale");
        return system.is_file().then_some(system);
    }
    crate::hearing::found("tailscale")
}

/// The address a port of the Host is reached at over HTTPS on Tailscale.
fn address_of(name: &str, port: u16) -> String {
    match port {
        443 => format!("https://{name}"),
        port => format!("https://{name}:{port}"),
    }
}

/// Has Tailscale serve what listens at `local` over HTTPS inside the
/// Owner's network, and gives the address it is then reached at. Asked
/// again, it gives the same address. The text says why it could not be.
pub fn open_on_tailscale(tailscale: &Path, local: SocketAddr) -> Result<String, String> {
    let status: Value = serde_json::from_str(&asked_of(tailscale, &["status", "--json"])?)
        .map_err(|_| "Tailscale's answer could not be read".to_owned())?;
    if status["BackendState"] != "Running" {
        return Err("Tailscale is not connected on this machine".into());
    }
    let Some(name) = status["CertDomains"][0].as_str() else {
        return Err("HTTPS certificates are not turned on for your Tailscale network (in its \
                    admin console: DNS, then HTTPS Certificates)"
            .into());
    };
    let target = format!("http://{local}");
    let serving: Value =
        serde_json::from_str(&asked_of(tailscale, &["serve", "status", "--json"])?)
            .unwrap_or_default();
    // Already served for Orbite: the same address again.
    for (place, served) in serving["Web"].as_object().into_iter().flatten() {
        if served["Handlers"]["/"]["Proxy"] == target.as_str()
            && let Some((_, port)) = place.rsplit_once(':')
            && let Ok(port) = port.parse()
        {
            return Ok(address_of(name, port));
        }
    }
    // A port nothing is served on yet: what the Owner already serves stays.
    let taken = |port: u16| serving["TCP"].get(port.to_string()).is_some();
    let port = (local.port()..local.port().saturating_add(20))
        .find(|port| !taken(*port))
        .ok_or("no free port was found to serve it on")?;
    asked_of(tailscale, &["serve", "--bg", &format!("--https={port}"), &target])?;
    Ok(address_of(name, port))
}

/// Has Tailscale stop serving what [`open_on_tailscale`] opened for what
/// listens at `local`, and nothing else: whatever is written down
/// elsewhere, only a port Tailscale itself says is served to there is
/// closed. Does nothing when there is none.
pub fn close_on_tailscale(tailscale: &Path, local: SocketAddr) -> Result<(), String> {
    let target = format!("http://{local}");
    let serving: Value =
        serde_json::from_str(&asked_of(tailscale, &["serve", "status", "--json"])?)
            .unwrap_or_default();
    for (place, served) in serving["Web"].as_object().into_iter().flatten() {
        if served["Handlers"]["/"]["Proxy"] == target.as_str()
            && let Some((_, port)) = place.rsplit_once(':')
            && let Ok(port) = port.parse::<u16>()
        {
            asked_of(tailscale, &["serve", &format!("--https={port}"), "off"])?;
        }
    }
    Ok(())
}
