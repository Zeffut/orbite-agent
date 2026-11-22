//! Reaching the web interface from the Owner's other devices: Tailscale,
//! where the Host has it, serves it to them over HTTPS. A stand-in for the
//! `tailscale` program answers here, and writes down what it was asked.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use orbite::{close_on_tailscale, open_on_tailscale};

/// A stand-in `tailscale` in `folder` that answers `status` and what it
/// `serves`, and writes each thing asked of it in `asked.log`.
fn tailscale_in(folder: &Path, status: &str, serves: &str) -> PathBuf {
    fs::write(folder.join("status.json"), status).unwrap();
    fs::write(folder.join("serves.json"), serves).unwrap();
    let program = folder.join("tailscale");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{0}/asked.log'\ncase \"$*\" in\n  'status --json') cat '{0}/status.json';;\n  \
         'serve status --json') cat '{0}/serves.json';;\nesac\n",
        folder.display()
    );
    fs::write(&program, script).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    program
}

const CONNECTED: &str = r#"{"BackendState":"Running","CertDomains":["host.tail1234.ts.net"]}"#;

fn asked(folder: &Path) -> Vec<String> {
    fs::read_to_string(folder.join("asked.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("serve ") && !line.contains("status"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn tailscale_serves_the_interface_over_https_inside_the_owners_network() {
    let folder = tempfile::tempdir().unwrap();
    let tailscale = tailscale_in(folder.path(), CONNECTED, "{}");

    let reached = open_on_tailscale(&tailscale, "127.0.0.1:8377".parse().unwrap()).unwrap();

    assert_eq!(reached, "https://host.tail1234.ts.net:8377");
    assert_eq!(asked(folder.path()), ["serve --bg --https=8377 http://127.0.0.1:8377"]);
}

#[test]
fn what_the_owner_already_serves_is_left_alone() {
    let folder = tempfile::tempdir().unwrap();
    let serves = r#"{"TCP":{"443":{"HTTPS":true},"8377":{"HTTPS":true}},
        "Web":{"host.tail1234.ts.net:8377":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}}"#;
    let tailscale = tailscale_in(folder.path(), CONNECTED, serves);

    let reached = open_on_tailscale(&tailscale, "127.0.0.1:8377".parse().unwrap()).unwrap();

    assert_eq!(reached, "https://host.tail1234.ts.net:8378");
    assert_eq!(asked(folder.path()), ["serve --bg --https=8378 http://127.0.0.1:8377"]);
}

#[test]
fn asked_again_the_same_address_is_given_and_nothing_more_is_served() {
    let folder = tempfile::tempdir().unwrap();
    let serves = r#"{"TCP":{"8378":{"HTTPS":true}},
        "Web":{"host.tail1234.ts.net:8378":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8377"}}}}}"#;
    let tailscale = tailscale_in(folder.path(), CONNECTED, serves);

    let reached = open_on_tailscale(&tailscale, "127.0.0.1:8377".parse().unwrap()).unwrap();

    assert_eq!(reached, "https://host.tail1234.ts.net:8378");
    assert!(asked(folder.path()).is_empty());
}

#[test]
fn a_tailscale_that_cannot_encrypt_is_said_so_and_asked_for_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let local = "127.0.0.1:8377".parse().unwrap();

    let tailscale = tailscale_in(folder.path(), r#"{"BackendState":"Stopped"}"#, "{}");
    assert!(open_on_tailscale(&tailscale, local).unwrap_err().contains("not connected"));
    let tailscale = tailscale_in(folder.path(), r#"{"BackendState":"Running"}"#, "{}");
    assert!(open_on_tailscale(&tailscale, local).unwrap_err().contains("HTTPS certificates"));

    assert!(asked(folder.path()).is_empty());
}

#[test]
fn what_tailscale_served_for_orbite_is_closed_by_its_port() {
    let folder = tempfile::tempdir().unwrap();
    let tailscale = tailscale_in(folder.path(), CONNECTED, "{}");

    close_on_tailscale(&tailscale, "https://host.tail1234.ts.net:8378").unwrap();

    assert_eq!(asked(folder.path()), ["serve --https=8378 off"]);
}

fn orbite(home: &Path, programs: &Path, arguments: &[&str]) -> String {
    let path = format!("{}:{}", programs.display(), std::env::var("PATH").unwrap_or_default());
    let output = Command::new(env!("CARGO_BIN_EXE_orbite"))
        .args(arguments)
        .env("ORBITE_HOME", home)
        .env("PATH", path)
        .output()
        .unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn turned_on_where_tailscale_is_the_interface_is_reached_encrypted_and_turned_off_it_is_closed() {
    let home = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    tailscale_in(folder.path(), CONNECTED, "{}");

    let on = orbite(home.path(), folder.path(), &["interface", "on"]);
    assert!(on.contains("https://host.tail1234.ts.net:8377"), "{on}");
    assert!(on.contains("encrypted"), "{on}");
    // Orbite itself listens on the machine alone.
    let config = orbite::Config::load(home.path()).unwrap();
    assert_eq!(config.interface.as_deref(), Some("127.0.0.1:8377"));
    // The code is to be typed at the address the devices reach.
    let pairing = orbite(home.path(), folder.path(), &["pair"]);
    assert!(pairing.contains("Open https://host.tail1234.ts.net:8377 on the device"), "{pairing}");

    orbite(home.path(), folder.path(), &["interface", "off"]);
    assert_eq!(
        asked(folder.path()),
        ["serve --bg --https=8377 http://127.0.0.1:8377", "serve --https=8377 off"]
    );
    let config = orbite::Config::load(home.path()).unwrap();
    assert_eq!((config.interface, config.interface_reached_at), (None, None));
}

#[test]
fn turned_on_where_tailscale_cannot_encrypt_the_interface_stays_on_the_machine_and_says_why() {
    let home = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    tailscale_in(folder.path(), r#"{"BackendState":"Running"}"#, "{}");

    let on = orbite(home.path(), folder.path(), &["interface", "on"]);

    assert!(on.contains("on this machine only"), "{on}");
    assert!(on.contains("HTTPS certificates are not turned on"), "{on}");
    assert_eq!(orbite::Config::load(home.path()).unwrap().interface_reached_at, None);
}
