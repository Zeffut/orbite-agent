//! The install script, run against a pretend machine: its own root folder,
//! and stand-ins for the system commands that record what they were asked.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Machine {
    root: tempfile::TempDir,
}

impl Machine {
    /// A Debian-like machine with systemd, on a common processor.
    fn debian() -> Self {
        let machine = Self { root: tempfile::tempdir().unwrap() };
        machine.write("etc/os-release", "ID=debian\nVERSION_ID=\"13\"\nPRETTY_NAME=\"Debian GNU/Linux 13\"\n");
        fs::create_dir_all(machine.path("run/systemd/system")).unwrap();
        fs::create_dir_all(machine.path("usr/local/bin")).unwrap();
        fs::create_dir_all(machine.path("etc/systemd/system")).unwrap();
        fs::create_dir_all(machine.path("var/lib")).unwrap();
        // Stand-ins for what changes the system: each records its arguments.
        for command in ["useradd", "systemctl", "chown", "apt-get", "runuser"] {
            machine.stand_in(command, "exit 0");
        }
        machine.stand_in("id", "[ \"$1\" = \"-u\" ] && [ -z \"$2\" ] && { echo 0; exit 0; }; exit 1");
        machine.stand_in("uname", "case \"$1\" in -m) echo x86_64;; -s) echo Linux;; *) echo Linux;; esac");
        machine.release("the orbite program");
        machine
    }

    fn path(&self, inside: &str) -> PathBuf {
        self.root.path().join(inside)
    }

    fn write(&self, inside: &str, content: &str) {
        let file = self.path(inside);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }

    fn stand_in(&self, command: &str, body: &str) {
        let log = self.path("calls.log");
        let script = format!("#!/bin/sh\necho \"{command} $*\" >> '{}'\n{body}\n", log.display());
        let file = self.path(&format!("stand-ins/{command}"));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, script).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Publishes a release holding `program`, signed with a key made for the test.
    fn release(&self, program: &str) {
        let release = self.path("release");
        fs::create_dir_all(&release).unwrap();
        fs::write(release.join("orbite-x86_64-linux"), program).unwrap();
        let run = |arguments: &[&str]| {
            let done = Command::new(arguments[0]).args(&arguments[1..]).current_dir(&release).output().unwrap();
            assert!(done.status.success(), "{arguments:?}: {}", String::from_utf8_lossy(&done.stderr));
            done
        };
        let sums = run(&["sha256sum", "orbite-x86_64-linux"]);
        fs::write(release.join("SHA256SUMS"), sums.stdout).unwrap();
        run(&["openssl", "genpkey", "-algorithm", "ed25519", "-out", "signing.key"]);
        run(&["openssl", "pkey", "-in", "signing.key", "-pubout", "-out", "signing.pub"]);
        run(&[
            "openssl", "pkeyutl", "-sign", "-inkey", "signing.key", "-rawin", "-in", "SHA256SUMS", "-out",
            "SHA256SUMS.sig",
        ]);
    }

    /// Runs the installer on this machine.
    fn install(&self) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
        let path = format!("{}:{}", self.path("stand-ins").display(), std::env::var("PATH").unwrap());
        Command::new("sh")
            .arg(script)
            .env("PATH", path)
            .env("ORBITE_ROOT", self.root.path())
            .env("ORBITE_RELEASE_URL", format!("file://{}", self.path("release").display()))
            .env("ORBITE_SIGNING_KEY", fs::read_to_string(self.path("release/signing.pub")).unwrap())
            .env("ORBITE_SKIP_SETUP", "1")
            .output()
            .unwrap()
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.path("calls.log")).unwrap_or_default()
    }
}

fn shown(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn a_supported_machine_gets_the_program_an_account_and_a_service_that_restarts() {
    let machine = Machine::debian();

    let installed = machine.install();

    assert!(installed.status.success(), "{}", shown(&installed));
    let program = machine.path("usr/local/bin/orbite");
    assert_eq!(fs::read_to_string(&program).unwrap(), "the orbite program");
    assert_eq!(fs::metadata(&program).unwrap().permissions().mode() & 0o777, 0o755);
    let service = fs::read_to_string(machine.path("etc/systemd/system/orbite.service")).unwrap();
    assert!(service.contains("User=orbite"), "{service}");
    assert!(service.contains("Restart=always"), "{service}");
    assert!(service.contains("ExecStart=/usr/local/bin/orbite run"), "{service}");
    assert!(service.contains("ORBITE_HOME=/var/lib/orbite"), "{service}");
    let calls = machine.calls();
    assert!(calls.contains("useradd") && calls.contains("orbite"), "{calls}");
    assert!(calls.contains("systemctl daemon-reload"), "{calls}");
    assert!(calls.contains("systemctl enable orbite"), "{calls}");
}

#[test]
fn an_unsupported_machine_is_refused_before_anything_is_changed() {
    let other_system = Machine::debian();
    other_system.write("etc/os-release", "ID=fedora\nVERSION_ID=43\nPRETTY_NAME=\"Fedora Linux 43\"\n");
    let other_processor = Machine::debian();
    other_processor.stand_in("uname", "case \"$1\" in -m) echo riscv64;; *) echo Linux;; esac");
    let no_systemd = Machine::debian();
    fs::remove_dir_all(no_systemd.path("run/systemd")).unwrap();
    let not_root = Machine::debian();
    not_root.stand_in("id", "[ \"$1\" = \"-u\" ] && [ -z \"$2\" ] && { echo 1000; exit 0; }; exit 1");

    for (machine, reason) in [
        (other_system, "Fedora"),
        (other_processor, "riscv64"),
        (no_systemd, "systemd"),
        (not_root, "root"),
    ] {
        let refused = machine.install();

        assert!(!refused.status.success(), "it was installed: {}", shown(&refused));
        assert!(shown(&refused).contains(reason), "the reason is said ({reason}): {}", shown(&refused));
        assert!(!machine.path("usr/local/bin/orbite").exists());
        assert!(!machine.path("etc/systemd/system/orbite.service").exists());
        let calls = machine.calls();
        assert!(!calls.contains("useradd") && !calls.contains("systemctl"), "something was changed: {calls}");
    }
}

#[test]
fn a_program_that_was_tampered_with_is_not_installed() {
    let altered = Machine::debian();
    fs::write(altered.path("release/orbite-x86_64-linux"), "something else").unwrap();
    let forged = Machine::debian();
    // The list of sums is rewritten to match another program, without the key.
    fs::write(forged.path("release/orbite-x86_64-linux"), "something else").unwrap();
    let sums = Command::new("sha256sum")
        .arg("orbite-x86_64-linux")
        .current_dir(forged.path("release"))
        .output()
        .unwrap();
    fs::write(forged.path("release/SHA256SUMS"), sums.stdout).unwrap();

    for machine in [altered, forged] {
        let refused = machine.install();

        assert!(!refused.status.success(), "{}", shown(&refused));
        assert!(shown(&refused).contains("not installed"), "{}", shown(&refused));
        assert!(!machine.path("usr/local/bin/orbite").exists());
    }
}

#[test]
fn installing_again_updates_the_program_and_keeps_what_the_owner_has() {
    let machine = Machine::debian();
    assert!(machine.install().status.success());
    machine.write("var/lib/orbite/config.json", "{\"agent_name\":\"Nova\"}");
    // The account exists now.
    machine.stand_in("id", "[ \"$1\" = \"-u\" ] && [ -z \"$2\" ] && { echo 0; exit 0; }; exit 0");
    machine.release("the orbite program, newer");
    fs::remove_file(machine.path("calls.log")).unwrap();

    let again = machine.install();

    assert!(again.status.success(), "{}", shown(&again));
    assert_eq!(fs::read_to_string(machine.path("usr/local/bin/orbite")).unwrap(), "the orbite program, newer");
    assert_eq!(fs::read_to_string(machine.path("var/lib/orbite/config.json")).unwrap(), "{\"agent_name\":\"Nova\"}");
    let calls = machine.calls();
    assert!(!calls.contains("useradd"), "the account was made twice: {calls}");
    assert!(calls.contains("systemctl restart orbite"), "the running service takes the new program: {calls}");
}

#[test]
fn a_machine_with_little_memory_is_warned_and_still_installed() {
    let machine = Machine::debian();
    machine.write("proc/meminfo", "MemTotal:        1000000 kB\n");

    let installed = machine.install();

    assert!(installed.status.success(), "{}", shown(&installed));
    assert!(shown(&installed).contains("memory"), "{}", shown(&installed));
}
