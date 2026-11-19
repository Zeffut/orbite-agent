//! What an Owner counts on when moving Orbite to another Host, or removing
//! it: an export restored elsewhere gives the same Agent, and uninstalling
//! leaves nothing behind but the data they asked to keep.

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use orbite::{Agent, Config, Secrets, Store};
use support::{ScriptedProvider, memory_channel};

fn orbite(home: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orbite"))
        .args(arguments)
        .env("ORBITE_HOME", home)
        .output()
        .unwrap()
}

fn shown(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A home an Agent has lived in: settings, a secret, a memory, a file of
/// its own and a Conversation.
async fn lived_in() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    Config { agent_name: Some("Nova".into()), language: Some("fr".into()), ..Config::default() }
        .save(home.path())
        .unwrap();
    Secrets::kept_in(home.path())
        .set("WEATHER_KEY", &["a", "key", "for", "the", "test"].join("-"))
        .unwrap();
    fs::create_dir_all(home.path().join("memory")).unwrap();
    fs::write(home.path().join("memory/owner.md"), "Thomas likes short answers.").unwrap();
    fs::create_dir_all(home.path().join("workspace/reports")).unwrap();
    fs::write(home.path().join("workspace/reports/week.md"), "All good.").unwrap();
    let (mut channel, mut owner) = memory_channel();
    let store = Store::open(home.path().join("orbite.db")).unwrap();
    let talking = tokio::spawn(async move {
        Agent::new(ScriptedProvider::answering(["Hello Thomas."]), store)
            .converse(&mut channel)
            .await
    });
    owner.says("Hello");
    owner.hears().await;
    drop(owner);
    talking.await.unwrap();
    home
}

#[tokio::test]
async fn an_export_restored_on_another_host_gives_the_same_agent() {
    let home = lived_in().await;
    let elsewhere = tempfile::tempdir().unwrap();
    let archive = elsewhere.path().join("orbite-export.tar.gz");

    let exported = orbite(home.path(), &["export", archive.to_str().unwrap()]);
    assert!(exported.status.success(), "{}", shown(&exported));
    // It holds the secrets and the key that opens them: for its Owner only.
    assert_eq!(fs::metadata(&archive).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(shown(&exported).contains("secrets"), "{}", shown(&exported));

    let fresh = elsewhere.path().join("home");
    let restored = orbite(&fresh, &["restore", archive.to_str().unwrap()]);
    assert!(restored.status.success(), "{}", shown(&restored));

    assert_eq!(Config::load(&fresh).unwrap(), Config::load(home.path()).unwrap());
    assert_eq!(
        Secrets::kept_in(&fresh).get("WEATHER_KEY").unwrap().as_deref(),
        Some("a-key-for-the-test")
    );
    assert_eq!(
        fs::read_to_string(fresh.join("memory/owner.md")).unwrap(),
        "Thomas likes short answers."
    );
    assert_eq!(fs::read_to_string(fresh.join("workspace/reports/week.md")).unwrap(), "All good.");
    let said: Vec<String> = Store::open(fresh.join("orbite.db"))
        .unwrap()
        .history(None, 10)
        .unwrap()
        .into_iter()
        .map(|entry| format!("{:?}", entry.message))
        .collect();
    assert!(said.len() == 2 && said[1].contains("Hello Thomas."), "{said:?}");
    assert_eq!(
        fs::metadata(fresh.join("secrets.key")).unwrap().permissions().mode() & 0o077,
        0,
        "the key is readable by others"
    );
}

#[tokio::test]
async fn restoring_never_overwrites_an_agent_that_is_already_there() {
    let home = lived_in().await;
    let other = lived_in().await;
    let archive = home.path().join("export.tar.gz");
    orbite(home.path(), &["export", archive.to_str().unwrap()]);
    fs::write(other.path().join("memory/owner.md"), "Someone else.").unwrap();

    let refused = orbite(other.path(), &["restore", archive.to_str().unwrap()]);

    assert!(!refused.status.success(), "{}", shown(&refused));
    assert!(shown(&refused).contains("already"), "{}", shown(&refused));
    assert_eq!(fs::read_to_string(other.path().join("memory/owner.md")).unwrap(), "Someone else.");
}

#[test]
fn a_file_that_is_not_an_export_is_refused_and_nothing_is_written() {
    let elsewhere = tempfile::tempdir().unwrap();
    let not_one = elsewhere.path().join("holiday.tar.gz");
    fs::write(&not_one, "not an archive").unwrap();
    let fresh = elsewhere.path().join("home");

    let refused = orbite(&fresh, &["restore", not_one.to_str().unwrap()]);

    assert!(!refused.status.success());
    assert!(!fresh.join("orbite.db").exists() && !fresh.join("config.json").exists());
}

/// A pretend machine with Orbite installed, and stand-ins that record what
/// they are asked.
struct Machine {
    root: tempfile::TempDir,
}

impl Machine {
    fn installed() -> Self {
        let machine = Self { root: tempfile::tempdir().unwrap() };
        for file in [
            "usr/local/bin/orbite",
            "usr/local/bin/orbite.previous",
            "etc/systemd/system/orbite.service",
            "etc/systemd/system/orbite-update.service",
            "etc/systemd/system/orbite-update.timer",
            "var/lib/orbite/orbite.db",
            "var/lib/orbite/memory/owner.md",
        ] {
            let path = machine.root.path().join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        for command in ["systemctl", "userdel", "chown"] {
            let script = format!(
                "#!/bin/sh\necho \"{command} $*\" >> '{}'\n",
                machine.root.path().join("calls.log").display()
            );
            let file = machine.root.path().join("stand-ins").join(command);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, script).unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        }
        machine
    }

    fn uninstall(&self, arguments: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.root.path().join("stand-ins").display(),
            std::env::var("PATH").unwrap()
        );
        Command::new(env!("CARGO_BIN_EXE_orbite"))
            .arg("uninstall")
            .args(arguments)
            .env("ORBITE_ROOT", self.root.path())
            .env("PATH", path)
            .output()
            .unwrap()
    }

    fn has(&self, file: &str) -> bool {
        self.root.path().join(file).exists()
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.root.path().join("calls.log")).unwrap_or_default()
    }
}

#[test]
fn uninstalling_removes_the_service_the_account_the_program_and_the_data() {
    let machine = Machine::installed();

    let removed = machine.uninstall(&["--yes"]);

    assert!(removed.status.success(), "{}", shown(&removed));
    for gone in [
        "usr/local/bin/orbite",
        "usr/local/bin/orbite.previous",
        "etc/systemd/system/orbite.service",
        "etc/systemd/system/orbite-update.service",
        "etc/systemd/system/orbite-update.timer",
        "var/lib/orbite",
    ] {
        assert!(!machine.has(gone), "{gone} was left behind");
    }
    let calls = machine.calls();
    for expected in [
        "systemctl disable --now orbite",
        "systemctl disable --now orbite-update.timer",
        "systemctl daemon-reload",
        "userdel orbite",
    ] {
        assert!(calls.contains(expected), "{expected} is missing from:\n{calls}");
    }
}

#[test]
fn the_data_is_kept_when_asked_and_the_owner_is_told_where() {
    let machine = Machine::installed();

    let removed = machine.uninstall(&["--yes", "--keep-data"]);

    assert!(removed.status.success(), "{}", shown(&removed));
    assert!(!machine.has("usr/local/bin/orbite") && machine.has("var/lib/orbite/memory/owner.md"));
    assert!(shown(&removed).contains("var/lib/orbite"), "{}", shown(&removed));
    // What is kept is given to root before the account goes: left to the
    // account's number, it would be read by whoever is given that number next.
    let calls = machine.calls();
    let given = calls.find("chown -R root:root").unwrap_or_else(|| panic!("{calls}"));
    assert!(calls[given..].lines().next().unwrap().ends_with("var/lib/orbite"), "{calls}");
    assert!(given < calls.find("userdel orbite").unwrap(), "{calls}");
}

#[test]
fn nothing_is_removed_without_the_owner_saying_yes() {
    let machine = Machine::installed();

    let asked = machine.uninstall(&[]);

    assert!(!asked.status.success());
    assert!(machine.has("usr/local/bin/orbite") && machine.has("var/lib/orbite/orbite.db"));
    assert!(machine.calls().is_empty(), "{}", machine.calls());
}

#[tokio::test]
async fn links_the_agent_made_are_kept_as_links_and_never_followed() {
    let home = lived_in().await;
    // As a Python environment leaves in a workspace: one to a program of the
    // system, one to something that is not there.
    std::os::unix::fs::symlink("/usr/bin/env", home.path().join("workspace/env")).unwrap();
    std::os::unix::fs::symlink("/nowhere/at/all", home.path().join("workspace/dangling")).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let archive = elsewhere.path().join("export.tar.gz");
    assert!(orbite(home.path(), &["export", archive.to_str().unwrap()]).status.success());

    let fresh = elsewhere.path().join("home");
    let restored = orbite(&fresh, &["restore", archive.to_str().unwrap()]);

    assert!(restored.status.success(), "{}", shown(&restored));
    assert_eq!(
        fs::read_link(fresh.join("workspace/dangling")).unwrap().to_str(),
        Some("/nowhere/at/all")
    );
    assert_eq!(fs::read_link(fresh.join("workspace/env")).unwrap().to_str(), Some("/usr/bin/env"));
}

#[tokio::test]
async fn an_export_written_inside_the_agents_folder_does_not_swallow_itself() {
    let home = lived_in().await;

    let exported = Command::new(env!("CARGO_BIN_EXE_orbite"))
        .args(["export", "backup.tar.gz"])
        .current_dir(home.path())
        .env("ORBITE_HOME", home.path())
        .output()
        .unwrap();

    assert!(exported.status.success(), "{}", shown(&exported));
    assert!(fs::metadata(home.path().join("backup.tar.gz")).unwrap().len() < 1_000_000);
    // Nothing of the Conversation is left lying beside it.
    let left: Vec<_> = fs::read_dir(home.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().contains("copy"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn a_restore_that_cannot_be_whole_changes_nothing() {
    let home = lived_in().await;
    let elsewhere = tempfile::tempdir().unwrap();
    let archive = elsewhere.path().join("export.tar.gz");
    orbite(home.path(), &["export", archive.to_str().unwrap()]);
    // No Agent here yet, but a folder of the same name as one in the export, with something in it.
    let fresh = elsewhere.path().join("home");
    fs::create_dir_all(fresh.join("workspace")).unwrap();
    fs::write(fresh.join("workspace/mine.txt"), "kept").unwrap();

    let refused = orbite(&fresh, &["restore", archive.to_str().unwrap()]);

    assert!(!refused.status.success(), "{}", shown(&refused));
    assert!(shown(&refused).contains("workspace"), "{}", shown(&refused));
    assert!(
        !fresh.join("config.json").exists() && !fresh.join("orbite.db").exists(),
        "it was restored in part"
    );
    assert_eq!(fs::read_to_string(fresh.join("workspace/mine.txt")).unwrap(), "kept");
}
