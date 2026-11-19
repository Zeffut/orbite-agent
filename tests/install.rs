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
        machine.write(
            "etc/os-release",
            "ID=debian\nVERSION_ID=\"13\"\nPRETTY_NAME=\"Debian GNU/Linux 13\"\n",
        );
        fs::create_dir_all(machine.path("run/systemd/system")).unwrap();
        fs::create_dir_all(machine.path("usr/local/bin")).unwrap();
        fs::create_dir_all(machine.path("etc/systemd/system")).unwrap();
        fs::create_dir_all(machine.path("var/lib")).unwrap();
        // Stand-ins for what changes the system: each records its arguments.
        for command in ["useradd", "systemctl", "chown", "apt-get", "runuser"] {
            machine.stand_in(command, "exit 0");
        }
        // What gpg says of Mozilla's own key.
        machine.stand_in("gpg", "echo 'fpr:::::::::35BAA0B33E9EB396F59CA838C0BA5CE6DC6315A3:'");
        machine
            .stand_in("id", "[ \"$1\" = \"-u\" ] && [ -z \"$2\" ] && { echo 0; exit 0; }; exit 1");
        machine.stand_in(
            "uname",
            "case \"$1\" in -m) echo x86_64;; -s) echo Linux;; *) echo Linux;; esac",
        );
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
            let done = Command::new(arguments[0])
                .args(&arguments[1..])
                .current_dir(&release)
                .output()
                .unwrap();
            assert!(
                done.status.success(),
                "{arguments:?}: {}",
                String::from_utf8_lossy(&done.stderr)
            );
            done
        };
        let sums = run(&["sha256sum", "orbite-x86_64-linux"]);
        fs::write(release.join("SHA256SUMS"), sums.stdout).unwrap();
        run(&["openssl", "genpkey", "-algorithm", "ed25519", "-out", "signing.key"]);
        run(&["openssl", "pkey", "-in", "signing.key", "-pubout", "-out", "signing.pub"]);
        run(&[
            "openssl",
            "pkeyutl",
            "-sign",
            "-inkey",
            "signing.key",
            "-rawin",
            "-in",
            "SHA256SUMS",
            "-out",
            "SHA256SUMS.sig",
        ]);
    }

    /// Runs the installer on this machine.
    fn install(&self) -> Output {
        self.install_with(&[])
    }

    /// Runs the installer with more said in its environment.
    fn install_with(&self, said: &[(&str, &str)]) -> Output {
        // Mozilla's key, as a file of this machine rather than fetched.
        self.write("mozilla.key", "a key");
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
        let path =
            format!("{}:{}", self.path("stand-ins").display(), std::env::var("PATH").unwrap());
        Command::new("sh")
            .arg(script)
            .env("PATH", path)
            .env("ORBITE_ROOT", self.root.path())
            .env("ORBITE_RELEASE_URL", format!("file://{}", self.path("release").display()))
            .env(
                "ORBITE_SIGNING_KEY",
                fs::read_to_string(self.path("release/signing.pub")).unwrap(),
            )
            .env("ORBITE_SKIP_SETUP", "1")
            .env("ORBITE_MOZILLA_KEY_URL", format!("file://{}", self.path("mozilla.key").display()))
            .envs(said.iter().copied())
            .output()
            .unwrap()
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.path("calls.log")).unwrap_or_default()
    }
}

fn shown(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
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
    // It keeps itself up to date: a timer runs the update, as root.
    let update =
        fs::read_to_string(machine.path("etc/systemd/system/orbite-update.service")).unwrap();
    assert!(update.contains("ExecStart=/usr/local/bin/orbite update"), "{update}");
    assert!(!update.contains("User=orbite"), "{update}");
    let timer = fs::read_to_string(machine.path("etc/systemd/system/orbite-update.timer")).unwrap();
    assert!(timer.contains("OnCalendar=daily") && timer.contains("Persistent=true"), "{timer}");
    assert!(calls.contains("systemctl enable --now orbite-update.timer"), "{calls}");
}

#[test]
fn an_unsupported_machine_is_refused_before_anything_is_changed() {
    let other_system = Machine::debian();
    other_system
        .write("etc/os-release", "ID=fedora\nVERSION_ID=43\nPRETTY_NAME=\"Fedora Linux 43\"\n");
    let other_processor = Machine::debian();
    other_processor.stand_in("uname", "case \"$1\" in -m) echo riscv64;; *) echo Linux;; esac");
    let no_systemd = Machine::debian();
    fs::remove_dir_all(no_systemd.path("run/systemd")).unwrap();
    let not_root = Machine::debian();
    not_root
        .stand_in("id", "[ \"$1\" = \"-u\" ] && [ -z \"$2\" ] && { echo 1000; exit 0; }; exit 1");

    for (machine, reason) in [
        (other_system, "Fedora"),
        (other_processor, "riscv64"),
        (no_systemd, "systemd"),
        (not_root, "root"),
    ] {
        let refused = machine.install();

        assert!(!refused.status.success(), "it was installed: {}", shown(&refused));
        assert!(
            shown(&refused).contains(reason),
            "the reason is said ({reason}): {}",
            shown(&refused)
        );
        assert!(!machine.path("usr/local/bin/orbite").exists());
        assert!(!machine.path("etc/systemd/system/orbite.service").exists());
        let calls = machine.calls();
        assert!(
            !calls.contains("useradd") && !calls.contains("systemctl"),
            "something was changed: {calls}"
        );
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
    assert_eq!(
        fs::read_to_string(machine.path("usr/local/bin/orbite")).unwrap(),
        "the orbite program, newer"
    );
    assert_eq!(
        fs::read_to_string(machine.path("var/lib/orbite/config.json")).unwrap(),
        "{\"agent_name\":\"Nova\"}"
    );
    let calls = machine.calls();
    assert!(!calls.contains("useradd"), "the account was made twice: {calls}");
    assert!(
        calls.contains("systemctl restart orbite"),
        "the running service takes the new program: {calls}"
    );
}

#[test]
fn a_machine_with_little_memory_is_warned_and_still_installed() {
    let machine = Machine::debian();
    machine.write("proc/meminfo", "MemTotal:        1000000 kB\n");

    let installed = machine.install();

    assert!(installed.status.success(), "{}", shown(&installed));
    assert!(shown(&installed).contains("memory"), "{}", shown(&installed));
}

#[test]
fn the_program_trusts_the_same_key_as_the_installer() {
    use base64::Engine;
    let installer = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh")).unwrap();
    // An ed25519 public key as the installer writes it: a fixed prefix, then the key.
    let mut written = hex("302a300506032b6570032100");
    written.extend_from_slice(&orbite::RELEASE_KEY);
    let written = base64::engine::general_purpose::STANDARD.encode(written);
    assert!(installer.contains(&written), "install.sh does not hold {written}");
}

fn hex(written: &str) -> Vec<u8> {
    (0..written.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&written[at..at + 2], 16).unwrap())
        .collect()
}

#[test]
fn the_agents_computer_is_set_up_with_a_screen_a_browser_and_fonts() {
    let machine = Machine::debian();

    let installed = machine.install();

    assert!(installed.status.success(), "{}", shown(&installed));
    let calls = machine.calls();
    let computer = calls
        .lines()
        .find(|call| call.starts_with("apt-get install") && call.contains("firefox"))
        .unwrap_or_else(|| panic!("the Computer's packages were not installed:\n{calls}"));
    for package in ["xvfb", "xdotool", "fonts-liberation", "fonts-noto-color-emoji"] {
        assert!(computer.contains(package), "{package} is missing from: {computer}");
    }
    // The browser comes from its maker's own packages, pinned so that the
    // system's own packaging of it is not taken instead.
    let source = fs::read_to_string(machine.path("etc/apt/sources.list.d/mozilla.list")).unwrap();
    assert!(
        source.contains("https://packages.mozilla.org/apt") && source.contains("signed-by="),
        "{source}"
    );
    assert_eq!(
        fs::read_to_string(machine.path("etc/apt/keyrings/packages.mozilla.org.asc")).unwrap(),
        "a key"
    );
    assert!(
        fs::read_to_string(machine.path("etc/apt/preferences.d/mozilla"))
            .unwrap()
            .contains("packages.mozilla.org")
    );
}

#[test]
fn the_computer_can_be_left_out_and_then_nothing_of_it_is_touched() {
    let machine = Machine::debian();

    let installed = machine.install_with(&[("ORBITE_WITHOUT_COMPUTER", "1")]);

    assert!(installed.status.success(), "{}", shown(&installed));
    assert!(!machine.calls().contains("firefox"), "{}", machine.calls());
    assert!(!machine.path("etc/apt/sources.list.d/mozilla.list").exists());
    assert!(
        shown(&installed).contains("computer"),
        "the Owner is told what the Agent goes without"
    );
}

#[test]
fn orbite_is_installed_even_when_the_computer_cannot_be() {
    let machine = Machine::debian();
    // The package manager fails on the browser, as when a mirror is down.
    machine.stand_in("apt-get", "case \"$*\" in *firefox*) exit 100;; esac; exit 0");

    let installed = machine.install();

    assert!(installed.status.success(), "{}", shown(&installed));
    assert!(machine.path("usr/local/bin/orbite").exists());
    assert!(shown(&installed).contains("computer could not be set up"), "{}", shown(&installed));
}

#[test]
fn installing_again_does_not_start_an_orbite_that_was_never_set_up() {
    let machine = Machine::debian();
    machine.install();
    fs::remove_file(machine.path("calls.log")).unwrap();

    let again = machine.install();

    assert!(again.status.success(), "{}", shown(&again));
    assert!(!machine.calls().contains("systemctl restart orbite"), "{}", machine.calls());
    assert!(shown(&again).contains("orbite setup"), "{}", shown(&again));
}

#[test]
fn a_machine_whose_openssl_cannot_check_the_signature_is_told_so_and_not_that_the_release_is_unsigned()
 {
    let machine = Machine::debian();
    // As OpenSSL 1.1.1 answers: it does not know how to check such a signature.
    machine.stand_in(
        "openssl",
        "[ \"$1\" = version ] && { echo 'OpenSSL 1.1.1w  11 Sep 2023'; exit 0; }; exit 1",
    );

    let refused = machine.install();

    assert!(!refused.status.success(), "{}", shown(&refused));
    assert!(shown(&refused).contains("OpenSSL 1.1.1w"), "{}", shown(&refused));
    assert!(shown(&refused).contains("OpenSSL 3"), "{}", shown(&refused));
    assert!(!shown(&refused).contains("not signed"), "{}", shown(&refused));
    assert!(!machine.path("usr/local/bin/orbite").exists());
}

#[test]
fn a_service_file_the_owner_edited_is_kept_aside_when_installing_again_and_they_are_told() {
    let machine = Machine::debian();
    assert!(machine.install().status.success());
    let service = machine.path("etc/systemd/system/orbite.service");
    let written = fs::read_to_string(&service).unwrap();
    // The file says where an Owner's own changes belong.
    assert!(written.contains("systemctl edit orbite"), "{written}");

    // Untouched, installing again says nothing and keeps nothing aside.
    let again = machine.install();
    assert!(!shown(&again).contains("orbite.service.replaced"), "{}", shown(&again));
    assert!(!machine.path("etc/systemd/system/orbite.service.replaced").exists());

    let edited =
        written.replace("RestartSec=3", "RestartSec=30\nEnvironment=ORBITE_CONTEXT_TOKENS=20000");
    fs::write(&service, &edited).unwrap();
    let again = machine.install();

    assert!(again.status.success(), "{}", shown(&again));
    assert_eq!(fs::read_to_string(&service).unwrap(), written);
    let aside = machine.path("etc/systemd/system/orbite.service.replaced");
    assert_eq!(fs::read_to_string(aside).unwrap(), edited);
    assert!(shown(&again).contains("orbite.service.replaced"), "{}", shown(&again));
    assert!(shown(&again).contains("systemctl edit orbite"), "{}", shown(&again));
}

#[test]
fn a_key_that_is_not_mozillas_is_not_trusted_for_the_browser() {
    let machine = Machine::debian();
    machine.stand_in("gpg", "echo 'fpr:::::::::0000000000000000000000000000000000000000:'");

    let installed = machine.install();

    // Orbite itself is installed; its Computer is not, and nothing is trusted.
    assert!(installed.status.success(), "{}", shown(&installed));
    assert!(shown(&installed).contains("is not Mozilla's"), "{}", shown(&installed));
    assert!(!machine.path("etc/apt/keyrings/packages.mozilla.org.asc").exists());
    assert!(!machine.calls().contains("firefox"), "{}", machine.calls());
}

#[test]
fn data_kept_from_an_earlier_installation_is_given_back_to_the_agents_account() {
    let machine = Machine::debian();
    machine.write("var/lib/orbite/memory/owner.md", "kept");

    assert!(machine.install().status.success());

    let calls = machine.calls();
    assert!(
        calls
            .lines()
            .any(|call| call.starts_with("chown -R orbite: ") && call.ends_with("var/lib/orbite")),
        "{calls}"
    );
}

#[test]
fn an_older_release_is_not_put_over_a_newer_orbite_unless_asked() {
    let machine = Machine::debian();
    // The release says which version it is, in its signed list.
    let release = machine.path("release");
    fs::write(release.join("VERSION"), "0.2.0\n").unwrap();
    let run = |arguments: &[&str]| {
        let done = Command::new(arguments[0])
            .args(&arguments[1..])
            .current_dir(&release)
            .output()
            .unwrap();
        assert!(done.status.success(), "{arguments:?}");
        done
    };
    let sums = run(&["sha256sum", "orbite-x86_64-linux", "VERSION"]);
    fs::write(release.join("SHA256SUMS"), sums.stdout).unwrap();
    run(&[
        "openssl",
        "pkeyutl",
        "-sign",
        "-inkey",
        "signing.key",
        "-rawin",
        "-in",
        "SHA256SUMS",
        "-out",
        "SHA256SUMS.sig",
    ]);
    // A newer Orbite runs on this machine.
    let installed = machine.path("usr/local/bin/orbite");
    fs::write(&installed, "#!/bin/sh\necho 'orbite 0.3.1'\n").unwrap();
    fs::set_permissions(&installed, fs::Permissions::from_mode(0o755)).unwrap();

    let refused = machine.install();

    assert!(!refused.status.success(), "{}", shown(&refused));
    assert!(shown(&refused).contains("0.3.1") && shown(&refused).contains("older 0.2.0"));
    assert!(fs::read_to_string(&installed).unwrap().contains("0.3.1"), "it was replaced");

    let insisted = machine.install_with(&[("ORBITE_ALLOW_OLDER", "1")]);
    assert!(insisted.status.success(), "{}", shown(&insisted));
    assert_eq!(fs::read_to_string(&installed).unwrap(), "the orbite program");
}

#[test]
fn a_system_made_from_debian_or_ubuntu_is_installed_as_they_are_and_told_so() {
    for about in [
        "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\nPRETTY_NAME=\"Linux Mint 22\"\n",
        "ID=kali\nID_LIKE=debian\nPRETTY_NAME=\"Kali GNU/Linux Rolling\"\n",
        "ID=raspbian\nID_LIKE=debian\nPRETTY_NAME=\"Raspbian GNU/Linux 12\"\n",
    ] {
        let machine = Machine::debian();
        machine.write("etc/os-release", about);

        let installed = machine.install();

        assert!(installed.status.success(), "{about}: {}", shown(&installed));
        assert!(shown(&installed).contains("made from Debian or Ubuntu"), "{}", shown(&installed));
        assert!(machine.path("usr/local/bin/orbite").exists());
    }
    // What only resembles them by name is not.
    let other = Machine::debian();
    other.write("etc/os-release", "ID=fedora\nID_LIKE=\"rhel centos\"\nPRETTY_NAME=\"Fedora\"\n");
    assert!(!other.install().status.success());
}

/// The address of the report offered in what was `shown`, taken apart.
fn report_offered(shown: &str) -> Option<std::collections::HashMap<String, String>> {
    let link = shown.split_whitespace().find(|word| word.contains("/issues/new?"))?;
    assert!(link.starts_with("https://github.com/Zeffut/orbite-agent/issues/new?"), "{link}");
    let decoded = |text: &str| {
        let mut bytes = Vec::new();
        let mut rest = text.as_bytes();
        while let Some((&first, after)) = rest.split_first() {
            if first == b'%' && after.len() >= 2 {
                let code = std::str::from_utf8(&after[..2]).unwrap();
                bytes.push(u8::from_str_radix(code, 16).unwrap());
                rest = &after[2..];
            } else {
                bytes.push(first);
                rest = after;
            }
        }
        String::from_utf8(bytes).unwrap()
    };
    let query = link.split_once('?').unwrap().1;
    Some(
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(name, value)| (name.to_owned(), decoded(value)))
            .collect(),
    )
}

#[test]
fn an_install_that_fails_unexpectedly_offers_a_report_the_owner_sends_themselves() {
    let machine = Machine::debian();
    // The account cannot be created, for a reason nobody foresaw.
    machine.stand_in("useradd", "echo 'useradd: cannot lock /etc/passwd' >&2; exit 1");

    let failed = machine.install();

    assert!(!failed.status.success());
    let shown = shown(&failed);
    let report = report_offered(&shown).unwrap_or_else(|| panic!("no report offered:\n{shown}"));
    // Nothing is sent: the link is theirs to open, and it says what it holds.
    assert!(shown.contains("Nothing is sent unless you open"), "{shown}");
    assert!(report["title"].contains("Install failed"), "{report:?}");
    let body = &report["body"];
    assert!(body.contains("Debian GNU/Linux 13") && body.contains("x86_64"), "{body}");
    assert!(body.contains("creating the agent's account"), "the step is named: {body}");
    // Nothing that names the person or the machine.
    for private in ["HOSTNAME", "/home/", "@"] {
        assert!(!body.contains(private), "{private} in {body}");
    }
}

#[test]
fn a_refusal_the_installer_explains_offers_no_report() {
    let other_system = Machine::debian();
    other_system.write("etc/os-release", "ID=fedora\nPRETTY_NAME=\"Fedora Linux 43\"\n");

    let refused = other_system.install();

    assert!(report_offered(&shown(&refused)).is_none(), "{}", shown(&refused));
}

#[test]
fn a_system_too_old_is_refused_before_anything_is_fetched_or_installed() {
    for about in [
        "ID=debian\nVERSION_ID=\"11\"\nPRETTY_NAME=\"Debian GNU/Linux 11 (bullseye)\"\n",
        "ID=ubuntu\nVERSION_ID=\"20.04\"\nPRETTY_NAME=\"Ubuntu 20.04.6 LTS\"\n",
    ] {
        let machine = Machine::debian();
        machine.write("etc/os-release", about);

        let refused = machine.install();

        assert!(!refused.status.success(), "{about}");
        assert!(shown(&refused).contains("is too old"), "{}", shown(&refused));
        assert!(!machine.calls().contains("apt-get"), "{}", machine.calls());
        assert!(report_offered(&shown(&refused)).is_none());
    }
    // The oldest ones taken, and one with no version at all (a rolling system).
    for about in [
        "ID=debian\nVERSION_ID=\"12\"\nPRETTY_NAME=\"Debian GNU/Linux 12\"\n",
        "ID=ubuntu\nVERSION_ID=\"22.04\"\nPRETTY_NAME=\"Ubuntu 22.04\"\n",
        "ID=debian\nPRETTY_NAME=\"Debian GNU/Linux forky/sid\"\n",
    ] {
        let machine = Machine::debian();
        machine.write("etc/os-release", about);
        assert!(machine.install().status.success(), "{about}");
    }
}

#[test]
fn what_is_installed_for_the_machine_is_readable_by_it_however_strict_the_account() {
    use std::os::unix::fs::PermissionsExt;
    let machine = Machine::debian();

    // Run by an account that makes its own files for itself only.
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    machine.write("mozilla.key", "a key");
    let path =
        format!("{}:{}", machine.path("stand-ins").display(), std::env::var("PATH").unwrap());
    let installed = Command::new("sh")
        .arg("-c")
        .arg(format!("umask 077; sh '{}'", script.display()))
        .env("PATH", path)
        .env("ORBITE_ROOT", machine.root.path())
        .env("ORBITE_RELEASE_URL", format!("file://{}", machine.path("release").display()))
        .env("ORBITE_SIGNING_KEY", fs::read_to_string(machine.path("release/signing.pub")).unwrap())
        .env("ORBITE_SKIP_SETUP", "1")
        .env("ORBITE_MOZILLA_KEY_URL", format!("file://{}", machine.path("mozilla.key").display()))
        .output()
        .unwrap();
    assert!(installed.status.success(), "{}", shown(&installed));

    // apt reads these as another account than root.
    for file in [
        "etc/apt/keyrings/packages.mozilla.org.asc",
        "etc/apt/sources.list.d/mozilla.list",
        "etc/apt/preferences.d/mozilla",
        "etc/systemd/system/orbite.service",
        "etc/systemd/system/orbite-update.timer",
    ] {
        let mode = fs::metadata(machine.path(file)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "{file}");
    }
    let keyrings = fs::metadata(machine.path("etc/apt/keyrings")).unwrap().permissions().mode();
    assert_eq!(keyrings & 0o777, 0o755);
    assert_eq!(
        fs::metadata(machine.path("usr/local/bin/orbite")).unwrap().permissions().mode() & 0o777,
        0o755
    );
}
