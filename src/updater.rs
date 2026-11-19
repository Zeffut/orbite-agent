//! Automatic updates: Orbite replaces its own program with a newer release,
//! but only one signed by Orbite's key, and puts the previous program back
//! when the new one does not start.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::Config;

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
            && Config::load(home).is_ok_and(|config| config.automatic_updates == Some(false))
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
        let refused = self.home.as_ref().map(|home| home.join("update-refused"));
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

        // The new program takes the place of the old in one move, and the
        // old one is kept beside it.
        let was_running = service.is_running();
        let arriving = sibling(&self.program, "new");
        let previous = sibling(&self.program, "previous");
        fs::write(&arriving, &program)
            .map_err(|error| failed("could not write the new program", error))?;
        make_runnable(&arriving)?;
        fs::copy(&self.program, &previous)
            .map_err(|error| failed("could not keep the previous program", error))?;
        fs::rename(&arriving, &self.program)
            .map_err(|error| failed("could not install the new program", error))?;

        // An Orbite that was not running (not set up yet) is not started,
        // and nothing can be judged from it staying stopped.
        if !was_running {
            return Ok(format!("Updated to {version}."));
        }
        service.restart().map_err(|error| failed("could not restart Orbite", error))?;
        tokio::time::sleep(self.wait).await;
        if service.is_running() {
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
            let _ = refused.parent().map(fs::create_dir_all);
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

    /// Leaves a word for the Owner, which the Agent passes on.
    fn tell(&self, text: &str) {
        let Some(home) = &self.home else { return };
        let notices = home.join("notices");
        if fs::create_dir_all(&notices).is_ok() {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_millis());
            let notice = notices.join(format!("{stamp}-update.txt"));
            let _ = fs::write(&notice, text);
            // Updates run as root, and the Agent, which is not root, has to
            // read the notice and remove it.
            #[cfg(unix)]
            if let Ok(owner) = fs::metadata(home) {
                use std::os::unix::fs::{MetadataExt, chown};
                for path in [&notices, &notice] {
                    let _ = chown(path, Some(owner.uid()), Some(owner.gid()));
                }
            }
        }
    }
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

/// The systemd service of an installed Orbite.
pub struct Systemd;

impl Service for Systemd {
    fn restart(&self) -> std::io::Result<()> {
        let done = std::process::Command::new("systemctl").args(["restart", "orbite"]).status()?;
        if done.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("systemctl could not restart orbite"))
        }
    }

    fn is_running(&self) -> bool {
        std::process::Command::new("systemctl")
            .args(["is-active", "--quiet", "orbite"])
            .status()
            .is_ok_and(|status| status.success())
    }
}
