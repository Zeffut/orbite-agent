//! The `orbite` command, run as an Owner would on the Host.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn orbite(home: &Path, arguments: &[&str], typed: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orbite"))
        .args(arguments)
        .env("ORBITE_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    command.stdin.take().unwrap().write_all(typed.as_bytes()).unwrap();
    command.wait_with_output().unwrap()
}

fn shown(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn a_secret_is_entered_listed_and_removed_on_the_host_without_ever_being_shown() {
    let home = tempfile::tempdir().unwrap();
    let value = "ghp_ExampleOnlyNotARealTokenValue0123456789";

    let set = orbite(home.path(), &["secret", "set", "github_token"], &format!("{value}\n"));
    assert!(set.status.success(), "{}", shown(&set));
    assert!(
        shown(&set).contains("GITHUB_TOKEN") && !shown(&set).contains(value),
        "{}",
        shown(&set)
    );

    let listed = orbite(home.path(), &["secret", "list"], "");
    assert_eq!(String::from_utf8_lossy(&listed.stdout).trim(), "GITHUB_TOKEN");

    assert_eq!(
        orbite::Secrets::kept_in(home.path()).get("GITHUB_TOKEN").unwrap().as_deref(),
        Some(value)
    );

    let removed = orbite(home.path(), &["secret", "remove", "GITHUB_TOKEN"], "");
    assert!(removed.status.success());
    assert!(
        String::from_utf8_lossy(&orbite(home.path(), &["secret", "list"], "").stdout)
            .trim()
            .is_empty()
    );
}

#[test]
fn what_is_wrong_with_a_secret_is_said_and_nothing_is_kept() {
    let home = tempfile::tempdir().unwrap();

    let unnamed = orbite(home.path(), &["secret", "set"], "value\n");
    assert!(!unnamed.status.success());
    assert!(shown(&unnamed).contains("orbite secret set NAME"), "{}", shown(&unnamed));

    let empty = orbite(home.path(), &["secret", "set", "TOKEN"], "\n");
    assert!(!empty.status.success());
    assert!(shown(&empty).contains("empty"), "{}", shown(&empty));

    let unknown = orbite(home.path(), &["secret", "remove", "NOTHING"], "");
    assert!(!unknown.status.success());
    assert!(orbite::Secrets::kept_in(home.path()).names().unwrap().is_empty());
}
