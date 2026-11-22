//! Automatic updates: Orbite replaces its own program with a newer release,
//! but only one signed by Orbite's key, and puts the previous program back
//! when the new one does not start.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

/// The key Orbite's releases are signed with (its public part): the same
/// one the installer holds.
pub const RELEASE_KEY: [u8; 32] = [
    0xe8, 0x56, 0x35, 0xe9, 0x71, 0xac, 0x2c, 0x8e, 0x8c, 0x04, 0x2c, 0x02, 0x54, 0xf7, 0xf0, 0x6c,
    0x37, 0x38, 0x1f, 0x66, 0xe1, 0x4f, 0xef, 0xcb, 0x32, 0x88, 0xf1, 0x4b, 0x84, 0x3f, 0x34, 0x48,
];

/// The service that runs the program being updated.
pub trait Service {
    fn restart(&self) -> std::io::Result<()>;
    fn is_running(&self) -> bool;
}

/// Why an update was not made, or was undone.
#[derive(Debug)]
pub struct UpdateError(String);

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for UpdateError {}

fn failed(what: &str, error: impl fmt::Display) -> UpdateError {
    UpdateError(format!("{what}: {error}"))
}

pub struct Updater {
    running: String,
    release: String,
    key: [u8; 32],
    program: PathBuf,
    home: Option<PathBuf>,
    wait: Duration,
    http: reqwest::Client,
}

impl Updater {
    /// Updates the Orbite of version `running` from the release published at
    /// `release`, trusting only what `key` signed.
    pub fn new(running: &str, release: impl Into<String>, key: [u8; 32]) -> Self {
        Self {
            running: running.to_owned(),
            release: release.into().trim_end_matches('/').to_owned(),
            key,
            program: PathBuf::from("/usr/local/bin/orbite"),
            home: None,
            wait: Duration::from_secs(15),
            http: reqwest::Client::new(),
        }
    }

    /// Where the program to replace is.
    pub fn installed_at(mut self, program: impl Into<PathBuf>) -> Self {
        self.program = program.into();
        self
    }

    /// The installation's folder: its settings say whether updates are
    /// wanted, and the Owner is told there of what was done.
    pub fn telling_the_owner_in(mut self, home: impl Into<PathBuf>) -> Self {
        self.home = Some(home.into());
        self
    }

    /// How long the service is given to start with the new program.
    pub fn waiting_for_the_service(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    /// Looks for a newer release and installs it. The text says what was done.
    pub async fn run(self, service: &impl Service) -> Result<String, UpdateError> {
        if let Some(home) = &self.home
            && updates_are_off(home)
        {
            return Ok("Automatic updates are off.".into());
        }

        // The list of checksums is what Orbite's key signs: nothing is
        // believed before that signature holds.
        let sums = self.download("SHA256SUMS").await?;
        let signature = self.download("SHA256SUMS.sig").await?;
        let key = VerifyingKey::from_bytes(&self.key)
            .map_err(|error| failed("Orbite's key is not a key", error))?;
        let signed = Signature::from_slice(&signature)
            .is_ok_and(|signature| key.verify(&sums, &signature).is_ok());
        if !signed {
            return Err(UpdateError(
                "the release is not signed by Orbite's key: nothing was changed".into(),
            ));
        }
        let sums = String::from_utf8_lossy(&sums).into_owned();

        let version = self.download("VERSION").await?;
        check(&sums, "VERSION", &version)?;
        let version = String::from_utf8_lossy(&version).trim().trim_start_matches('v').to_owned();
        if !is_newer(&version, &self.running) {
            return Ok(format!("Orbite {} is up to date.", self.running));
        }

        // A version that did not start here is left alone until the next one.
        // Kept beside the program, where only root writes: the Agent's own
        // folder is the Agent's, and nothing it could plant there is trusted.
        let refused = Some(sibling(&self.program, "refused"));
        if let Some(refused) = &refused
            && fs::read_to_string(refused).is_ok_and(|kept| kept.trim() == version)
        {
            return Ok(format!(
                "Version {version} did not start here before: waiting for the next one."
            ));
        }

        let file = format!("orbite-{}-linux", std::env::consts::ARCH);
        let program = self.download(&file).await?;
        check(&sums, &file, &program)?;

        // One update at a time: the daily one and one asked by hand would
        // write the same files.
        let lock = fs::File::create(sibling(&self.program, "updating"))
            .map_err(|error| failed("could not prepare the update", error))?;
        {
            use std::os::fd::AsRawFd;
            // SAFETY: a lock on a file this function just opened, let go when it is closed.
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Ok("Another update is under way.".into());
            }
        }
        // The new program takes the place of the old in one move, and the
        // old one is kept beside it.
        let was_running = service.is_running();
        let arriving = sibling(&self.program, "new");
        let previous = sibling(&self.program, "previous");
        // Written through to the disk before it takes the old one's place:
        // a machine that loses power just after must not find half a program.
        fs::File::create(&arriving)
            .and_then(|mut file| {
                use std::io::Write;
                file.write_all(&program)?;
                file.sync_all()
            })
            .map_err(|error| failed("could not write the new program", error))?;
        make_runnable(&arriving)?;
        fs::copy(&self.program, &previous)
            .map_err(|error| failed("could not keep the previous program", error))?;
        fs::rename(&arriving, &self.program)
            .map_err(|error| failed("could not install the new program", error))?;
        if let Some(folder) = self.program.parent()
            && let Ok(folder) = fs::File::open(folder)
        {
            let _ = folder.sync_all();
        }

        // An Orbite that was not running (not set up yet) is not started,
        // and nothing can be judged from it staying stopped.
        if !was_running {
            return Ok(format!("Updated to {version}."));
        }
        service.restart().map_err(|error| failed("could not restart Orbite", error))?;
        tokio::time::sleep(self.wait).await;
        // Looked at more than once: a program that starts and falls over is
        // started again by its service, and one look could catch it standing.
        let mut running = service.is_running();
        for _ in 0..2 {
            tokio::time::sleep(self.wait / 4).await;
            running &= service.is_running();
        }
        if running {
            self.tell(&format!(
                "Orbite was updated from version {} to version {version}.",
                self.running
            ));
            return Ok(format!("Updated to {version}."));
        }

        // It does not start: the previous program goes back.
        fs::copy(&previous, &arriving).and_then(|_| fs::rename(&arriving, &self.program)).map_err(
            |error| {
                failed(
                    "the update does not start, and the previous program could not be put back",
                    error,
                )
            },
        )?;
        let _ = service.restart();
        if let Some(refused) = &refused {
            let _ = fs::write(refused, &version);
        }
        self.tell(&format!(
            "An update of Orbite to version {version} did not start and was undone: version {} is running \
             again. Nothing was lost.",
            self.running
        ));
        Err(UpdateError(format!(
            "version {version} did not start: version {} was put back and is running",
            self.running
        )))
    }

    async fn download(&self, name: &str) -> Result<Vec<u8>, UpdateError> {
        let what = format!("could not download {name}");
        let response = self
            .http
            .get(format!("{}/{name}", self.release))
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| failed(&what, error))?;
        Ok(response.bytes().await.map_err(|error| failed(&what, error))?.to_vec())
    }

    /// Leaves a word for the Owner, which the Agent passes on. Updates run
    /// as root and the folder is the Agent's: nothing there is followed if
    /// it is a link, and nothing changes hands but a folder made here.
    fn tell(&self, text: &str) {
        use std::io::Write;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let Some(home) = &self.home else { return };
        let notices = home.join("notices");
        let made = fs::create_dir_all(home).is_ok() && fs::create_dir(&notices).is_ok();
        let opened = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&notices);
        let Ok(folder) = opened else { return };
        if made && let Ok(owner) = fs::symlink_metadata(home) {
            // The Agent, which is not root, has to remove what it has read.
            // SAFETY: a call on a folder this function holds open.
            unsafe { libc::fchown(folder.as_raw_fd(), owner.uid(), owner.gid()) };
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        let Ok(name) = std::ffi::CString::new(format!("{stamp}-update.txt")) else { return };
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW;
        // SAFETY: a new file, made inside the folder held open and nowhere else.
        let made = unsafe { libc::openat(folder.as_raw_fd(), name.as_ptr(), flags, 0o644) };
        if made >= 0 {
            // SAFETY: the descriptor was just opened and belongs to nothing else.
            let mut notice = unsafe { fs::File::from_raw_fd(made) };
            let _ = notice.write_all(text.as_bytes());
        }
    }
}

/// True when the settings kept in `home` say the Owner wants no automatic
/// update. Read as root from a folder that is not root's: only a plain file
/// of an ordinary size is read, and never through a link.
fn updates_are_off(home: &Path) -> bool {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let opened = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(home.join("config.json"));
    let Ok(file) = opened else { return false };
    if !file.metadata().is_ok_and(|about| about.is_file() && about.len() < 1_000_000) {
        return false;
    }
    let mut written = String::new();
    if file.take(1_000_000).read_to_string(&mut written).is_err() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(&written)
        .is_ok_and(|settings| settings["automatic_updates"] == false)
}

/// Fails unless `content` is what the signed list `sums` says `name` is.
fn check(sums: &str, name: &str, content: &[u8]) -> Result<(), UpdateError> {
    let actual: String = Sha256::digest(content).iter().map(|byte| format!("{byte:02x}")).collect();
    let expected = sums.lines().find_map(|line| {
        let (sum, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then_some(sum)
    });
    if expected == Some(actual.as_str()) {
        Ok(())
    } else {
        Err(UpdateError(format!("{name} does not match its signed checksum: nothing was changed")))
    }
}

/// True when version `candidate` comes after version `current`, each read as
/// numbers separated by dots.
fn is_newer(candidate: &str, current: &str) -> bool {
    let numbers = |version: &str| -> Vec<u64> {
        version
            .trim()
            .trim_start_matches('v')
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    };
    numbers(candidate) > numbers(current)
}

/// The file beside `program` with `suffix` added to its name.
fn sibling(program: &Path, suffix: &str) -> PathBuf {
    let mut name = program.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{suffix}"));
    program.with_file_name(name)
}

fn make_runnable(program: &Path) -> Result<(), UpdateError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(program, fs::Permissions::from_mode(0o755))
            .map_err(|error| failed("could not make the new program runnable", error))?;
    }
    Ok(())
}

/// Where the system keeps its own programs: nothing an account without
/// root's rights can write in.
const SYSTEM_FOLDERS: [&str; 6] =
    ["/usr/sbin", "/usr/bin", "/sbin", "/bin", "/usr/local/sbin", "/usr/local/bin"];

/// The program called `name` among `folders`, when one of them has it.
fn found_among(name: &str, folders: &[&str]) -> Option<std::path::PathBuf> {
    folders.iter().map(|folder| std::path::Path::new(folder).join(name)).find(|file| file.is_file())
}

/// A program of the system, to run. As root it is taken from where the
/// system keeps its own and nowhere else: whoever had Orbite run as root
/// has no say, through the folders they search, in what root then runs.
pub fn system_program(name: &str) -> std::path::PathBuf {
    // SAFETY: a plain system call.
    #[cfg(unix)]
    if unsafe { libc::geteuid() } == 0 {
        // Found nowhere there, it is not looked for anywhere else.
        return found_among(name, &SYSTEM_FOLDERS)
            .unwrap_or_else(|| std::path::Path::new(SYSTEM_FOLDERS[1]).join(name));
    }
    name.into()
}

/// True when `program` is what sudo itself was asked to run, by an account
/// that is not root: `asked_by` and `ran` are what sudo says of that
/// (`SUDO_UID`, `SUDO_COMMAND`). A root shell opened with sudo, in which the
/// program is then run, is root's own doing and is not this.
pub fn run_by_sudo_for_another(
    asked_by: Option<&str>,
    ran: Option<&str>,
    program: &std::path::Path,
) -> bool {
    let another = asked_by.is_some_and(|asked_by| asked_by.trim() != "0");
    let first = ran.and_then(|ran| ran.split_whitespace().next()).map(std::path::Path::new);
    another && first.is_some_and(|first| first.file_name() == program.file_name())
}

/// The systemd service of an installed Orbite.
pub struct Systemd;

impl Service for Systemd {
    fn restart(&self) -> std::io::Result<()> {
        // A program that kept failing may have used up the starts systemd
        // allows in a row: the count begins again with this one.
        let _ = std::process::Command::new(system_program("systemctl"))
            .args(["reset-failed", "orbite"])
            .status();
        let done = std::process::Command::new(system_program("systemctl"))
            .args(["restart", "orbite"])
            .status()?;
        if done.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("systemctl could not restart orbite"))
        }
    }

    fn is_running(&self) -> bool {
        std::process::Command::new(system_program("systemctl"))
            .args(["is-active", "--quiet", "orbite"])
            .status()
            .is_ok_and(|status| status.success())
    }
}

#[cfg(test)]
mod system_programs {
    use super::{found_among, run_by_sudo_for_another};

    #[test]
    fn only_a_program_sudo_itself_ran_for_another_account_counts_as_such() {
        let program = std::path::Path::new("/usr/local/bin/orbite");
        let by = |asked_by, ran| run_by_sudo_for_another(asked_by, ran, program);

        // The agent's account, through the rule that lets it.
        assert!(by(Some("998"), Some("/usr/local/bin/orbite update")));
        // A root shell opened with sudo, where the Owner then runs it.
        assert!(!by(Some("1000"), Some("/usr/bin/sh release/try-install.sh")));
        assert!(!by(Some("1000"), Some("/bin/bash")));
        // Root's own sudo, and no sudo at all.
        assert!(!by(Some("0"), Some("/usr/local/bin/orbite update")));
        assert!(!by(None, None));
    }

    #[test]
    fn a_program_is_taken_from_the_folders_named_and_from_no_other() {
        let system = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(system.path().join("systemctl"), "").unwrap();
        std::fs::write(elsewhere.path().join("tailscale"), "").unwrap();
        let folders = [system.path().to_str().unwrap()];

        assert_eq!(found_among("systemctl", &folders), Some(system.path().join("systemctl")));
        assert_eq!(found_among("tailscale", &folders), None);
    }
}
