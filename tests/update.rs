//! What an Owner counts on from automatic updates: only what Orbite's key
//! signed is installed, and an update that does not start is undone.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signer, SigningKey};
use orbite::{Service, Updater};
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A key made for the test, standing in for Orbite's.
fn key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// A published release: its version, its program, and whether to tamper.
struct Release {
    version: &'static str,
    program: &'static str,
    signed_by: SigningKey,
    program_served: Option<&'static str>,
}

impl Release {
    fn new(version: &'static str, program: &'static str) -> Self {
        Self { version, program, signed_by: key(), program_served: None }
    }

    async fn published(&self) -> MockServer {
        let server = MockServer::start().await;
        let file = format!("orbite-{}-linux", std::env::consts::ARCH);
        let sum = |content: &str| {
            Sha256::digest(content.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let sums = format!("{}  {file}\n{}  VERSION\n", sum(self.program), sum(self.version));
        let signature = self.signed_by.sign(sums.as_bytes()).to_bytes().to_vec();
        let served = self.program_served.unwrap_or(self.program);
        for (name, body) in [
            ("VERSION", self.version.as_bytes().to_vec()),
            ("SHA256SUMS", sums.into_bytes()),
            ("SHA256SUMS.sig", signature),
            (file.as_str(), served.as_bytes().to_vec()),
        ] {
            Mock::given(method("GET"))
                .and(path(format!("/{name}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
                .mount(&server)
                .await;
        }
        server
    }
}

/// A service that records what it is asked and starts or not as told.
#[derive(Clone)]
struct Pretend {
    asked: Arc<Mutex<Vec<String>>>,
    program: PathBuf,
    /// Programs that do not start.
    broken: &'static str,
}

impl Service for Pretend {
    fn restart(&self) -> std::io::Result<()> {
        self.asked.lock().unwrap().push("restart".into());
        Ok(())
    }

    fn is_running(&self) -> bool {
        fs::read_to_string(&self.program).is_ok_and(|program| program != self.broken)
    }
}

struct Machine {
    folder: tempfile::TempDir,
    service: Pretend,
}

impl Machine {
    /// A machine running `program` as Orbite `version`.
    fn running(program: &str) -> Self {
        let folder = tempfile::tempdir().unwrap();
        let installed = folder.path().join("orbite");
        fs::write(&installed, program).unwrap();
        let service = Pretend {
            asked: Arc::default(),
            program: installed,
            broken: "a program that does not start",
        };
        Self { folder, service }
    }

    fn program(&self) -> String {
        fs::read_to_string(self.folder.path().join("orbite")).unwrap()
    }

    fn home(&self) -> PathBuf {
        self.folder.path().join("home")
    }

    async fn update_from(&self, release: &MockServer, running: &str) -> Result<String, String> {
        Updater::new(running, release.uri(), key().verifying_key().to_bytes())
            .installed_at(self.folder.path().join("orbite"))
            .telling_the_owner_in(self.home())
            .waiting_for_the_service(std::time::Duration::from_millis(10))
            .run(&self.service)
            .await
            .map_err(|error| error.to_string())
    }

    fn notices(&self) -> Vec<String> {
        let folder = self.home().join("notices");
        let Ok(files) = fs::read_dir(folder) else { return Vec::new() };
        files.map(|file| fs::read_to_string(file.unwrap().path()).unwrap()).collect()
    }

    fn restarts(&self) -> usize {
        self.service.asked.lock().unwrap().len()
    }
}

fn exists(path: &Path) -> bool {
    path.exists()
}

#[tokio::test]
async fn a_newer_signed_release_replaces_the_program_and_the_service_takes_it() {
    let machine = Machine::running("the old program");
    let release = Release::new("0.2.0", "the new program").published().await;

    let done = machine.update_from(&release, "0.1.0").await.unwrap();

    assert!(done.contains("0.2.0"), "{done}");
    assert_eq!(machine.program(), "the new program");
    assert_eq!(machine.restarts(), 1);
    assert_eq!(
        fs::read_to_string(machine.folder.path().join("orbite.previous")).unwrap(),
        "the old program"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_new_program_can_be_run() {
    use std::os::unix::fs::PermissionsExt;
    let machine = Machine::running("the old program");
    let release = Release::new("0.2.0", "the new program").published().await;

    machine.update_from(&release, "0.1.0").await.unwrap();

    let mode = fs::metadata(machine.folder.path().join("orbite")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
}

#[tokio::test]
async fn nothing_happens_when_the_release_is_not_newer() {
    for published in ["0.1.0", "0.0.9"] {
        let machine = Machine::running("the old program");
        let release = Release::new(published, "another program").published().await;

        let done = machine.update_from(&release, "0.1.0").await.unwrap();

        assert!(done.contains("up to date"), "{done}");
        assert_eq!(machine.program(), "the old program");
        assert_eq!(machine.restarts(), 0);
    }
}

#[tokio::test]
async fn versions_are_compared_as_numbers_not_as_text() {
    let machine = Machine::running("the old program");
    let release = Release::new("0.10.0", "the new program").published().await;

    machine.update_from(&release, "0.9.0").await.unwrap();

    assert_eq!(machine.program(), "the new program");
}

#[tokio::test]
async fn a_release_signed_by_another_key_is_refused() {
    let machine = Machine::running("the old program");
    let forged = Release {
        signed_by: SigningKey::from_bytes(&[9u8; 32]),
        ..Release::new("0.2.0", "a forged program")
    };
    let release = forged.published().await;

    let refused = machine.update_from(&release, "0.1.0").await.unwrap_err();

    assert!(refused.contains("signed"), "{refused}");
    assert_eq!(machine.program(), "the old program");
    assert_eq!(machine.restarts(), 0);
}

#[tokio::test]
async fn a_program_that_does_not_match_its_signed_checksum_is_refused() {
    let machine = Machine::running("the old program");
    let swapped = Release {
        program_served: Some("a swapped program"),
        ..Release::new("0.2.0", "the new program")
    };
    let release = swapped.published().await;

    let refused = machine.update_from(&release, "0.1.0").await.unwrap_err();

    assert!(refused.contains("checksum"), "{refused}");
    assert_eq!(machine.program(), "the old program");
    assert!(!exists(&machine.folder.path().join("orbite.previous")));
}

#[tokio::test]
async fn an_update_that_does_not_start_is_undone_and_the_owner_is_told() {
    let machine = Machine::running("the old program");
    let release = Release::new("0.2.0", "a program that does not start").published().await;

    let undone = machine.update_from(&release, "0.1.0").await.unwrap_err();

    assert!(undone.contains("0.2.0") && undone.contains("back"), "{undone}");
    assert_eq!(machine.program(), "the old program", "the previous program is back");
    assert_eq!(machine.restarts(), 2, "once for the new program, once for the old one back");
    let notices = machine.notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("0.2.0") && notices[0].contains("0.1.0"), "{notices:?}");
}

#[tokio::test]
async fn a_version_that_did_not_start_is_not_tried_again() {
    let machine = Machine::running("the old program");
    let release = Release::new("0.2.0", "a program that does not start").published().await;
    machine.update_from(&release, "0.1.0").await.unwrap_err();

    let again = machine.update_from(&release, "0.1.0").await.unwrap();

    assert!(again.contains("0.2.0") && again.contains("did not start"), "{again}");
    assert_eq!(machine.restarts(), 2, "the service was disturbed again");
    assert_eq!(machine.notices().len(), 1, "the Owner was told twice");
}

#[tokio::test]
async fn the_owner_is_told_of_an_update_that_went_well() {
    let machine = Machine::running("the old program");
    let release = Release::new("0.2.0", "the new program").published().await;

    machine.update_from(&release, "0.1.0").await.unwrap();

    assert!(machine.notices()[0].contains("0.2.0"));
}

#[tokio::test]
async fn an_owner_who_turned_updates_off_is_left_alone() {
    let machine = Machine::running("the old program");
    orbite::Config { automatic_updates: Some(false), ..orbite::Config::default() }
        .save(machine.home())
        .unwrap();
    let release = Release::new("0.2.0", "the new program").published().await;

    let done = machine.update_from(&release, "0.1.0").await.unwrap();

    assert!(done.contains("off"), "{done}");
    assert_eq!(machine.program(), "the old program");
}

#[tokio::test]
async fn a_link_planted_where_orbite_leaves_word_is_never_followed() {
    // Updates run as root, and the Agent's folder is the Agent's: a link
    // there must not lead root anywhere else.
    let machine = Machine::running("the old program");
    let elsewhere = machine.folder.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(machine.home()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, machine.home().join("notices")).unwrap();
    fs::write(elsewhere.join("precious"), "untouched").unwrap();
    std::os::unix::fs::symlink(elsewhere.join("precious"), machine.home().join("update-refused"))
        .unwrap();
    let release = Release::new("0.2.0", "a program that does not start").published().await;

    machine.update_from(&release, "0.1.0").await.unwrap_err();

    let found: Vec<_> =
        fs::read_dir(&elsewhere).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(found.len(), 1, "something was written through the link: {found:?}");
    assert_eq!(fs::read_to_string(elsewhere.join("precious")).unwrap(), "untouched");
    assert_eq!(machine.program(), "the old program");
}

#[tokio::test]
async fn settings_that_are_not_a_plain_file_do_not_hold_the_update_up() {
    let machine = Machine::running("the old program");
    fs::create_dir_all(machine.home()).unwrap();
    // Something that would never finish being read.
    std::os::unix::fs::symlink("/dev/zero", machine.home().join("config.json")).unwrap();
    let release = Release::new("0.2.0", "the new program").published().await;

    let updated = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        machine.update_from(&release, "0.1.0"),
    )
    .await;

    assert!(updated.expect("the update hung on the settings").is_ok());
    assert_eq!(machine.program(), "the new program");
}

#[tokio::test]
async fn two_updates_at_once_do_not_get_in_each_others_way() {
    use std::os::fd::AsRawFd;
    let machine = Machine::running("the old program");
    let release = Release::new("0.2.0", "the new program").published().await;
    // Another update holds the program: as the daily one does while one is asked by hand.
    let held = fs::File::create(machine.folder.path().join("orbite.updating")).unwrap();
    // SAFETY: a lock taken on a file this test just made.
    assert_eq!(unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);

    let said = machine.update_from(&release, "0.1.0").await.unwrap();

    assert!(said.contains("under way"), "{said}");
    assert_eq!(machine.program(), "the old program");
    assert_eq!(machine.restarts(), 0);
}
