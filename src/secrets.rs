//! Secrets the Agent uses without the model ever seeing them: sealed on the
//! Host, given to a command by name, and hidden from whatever goes back to
//! the model.
//!
//! The key that seals them is a file beside them, readable by the Agent's
//! account only. This keeps secrets out of backups, logs and the model; it
//! does not protect them from someone who has that account.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};

/// The shortest value worth hiding: anything shorter would hide ordinary words.
const SHORTEST_HIDDEN: usize = 6;
/// The shortest part of a secret that is hidden wherever it shows.
const SHORTEST_PART_HIDDEN: usize = 12;

pub struct Secrets {
    sealed: PathBuf,
    key: PathBuf,
}

/// Why secrets could not be read or written.
#[derive(Debug)]
pub struct SecretsError(String);

impl fmt::Display for SecretsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SecretsError {}

fn failed(what: &str, error: impl fmt::Display) -> SecretsError {
    SecretsError(format!("{what}: {error}"))
}

impl Secrets {
    /// The secrets of the installation whose folder is `home`.
    pub fn kept_in(home: impl AsRef<Path>) -> Self {
        let home = home.as_ref();
        Self { sealed: home.join("secrets.sealed"), key: home.join("secrets.key") }
    }

    /// Keeps `value` under `name`, replacing what was kept there. Names are
    /// those of environment variables, in capitals.
    pub fn set(&self, name: &str, value: &str) -> Result<String, SecretsError> {
        let name = name.trim().to_uppercase();
        let well_named = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && name.chars().next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_');
        if !well_named {
            return Err(SecretsError(
                "a secret is named like an environment variable: letters, digits and _, not starting with a digit"
                    .into(),
            ));
        }
        let _alone = self.alone()?;
        let mut all = self.all()?;
        all.insert(name.clone(), value.to_owned());
        self.write(&all)?;
        Ok(name)
    }

    /// Alone to change what is kept, in this program and in any other of
    /// the installation: two changes at once would lose one of them.
    fn alone(&self) -> Result<fs::File, SecretsError> {
        use std::os::fd::AsRawFd;
        if let Some(folder) = self.sealed.parent() {
            fs::create_dir_all(folder)
                .map_err(|error| failed("could not create the folder", error))?;
        }
        let lock = fs::File::create(self.sealed.with_extension("lock"))
            .map_err(|error| failed("could not write", error))?;
        // SAFETY: a lock on a file opened just above, let go when it is closed.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(failed("could not write", std::io::Error::last_os_error()));
        }
        Ok(lock)
    }

    pub fn get(&self, name: &str) -> Result<Option<String>, SecretsError> {
        Ok(self.all()?.remove(&name.trim().to_uppercase()))
    }

    /// The names of the secrets kept, in order.
    pub fn names(&self) -> Result<Vec<String>, SecretsError> {
        Ok(self.all()?.into_keys().collect())
    }

    /// Deletes the secret called `name`; false when there is none.
    pub fn remove(&self, name: &str) -> Result<bool, SecretsError> {
        let _alone = self.alone()?;
        let mut all = self.all()?;
        let removed = all.remove(&name.trim().to_uppercase()).is_some();
        if removed {
            self.write(&all)?;
        }
        Ok(removed)
    }

    /// `text` with every secret's value replaced by its name, so that it can
    /// be shown to the model.
    pub(crate) fn hide(&self, text: &str) -> String {
        let Ok(all) = self.all() else { return text.to_owned() };
        let mut hidden = text.to_owned();
        for (name, value) in &all {
            if value.len() >= SHORTEST_HIDDEN {
                hidden = hidden.replace(value.as_str(), &format!("[secret {name}]"));
            }
        }
        // A large part of one says nearly as much as the whole: a command
        // may print only the beginning of a secret, or its end.
        for (name, value) in &all {
            hidden = without_parts_of(&hidden, value, &format!("[part of secret {name}]"));
        }
        hidden
    }

    /// The secrets `command` names, each as (name, value), to give it as
    /// environment variables. Orbite's own are never among them.
    pub(crate) fn named_in(&self, command: &str) -> Vec<(String, String)> {
        let Ok(all) = self.all() else { return Vec::new() };
        all.into_iter()
            .filter(|(name, _)| !is_orbites_own(name) && command.contains(name.as_str()))
            .collect()
    }

    /// The names of the secrets the Agent may use: all but Orbite's own.
    pub(crate) fn names_for_the_agent(&self) -> Result<Vec<String>, SecretsError> {
        Ok(self.names()?.into_iter().filter(|name| !is_orbites_own(name)).collect())
    }

    fn cipher(&self) -> Result<ChaCha20Poly1305, SecretsError> {
        let key = match fs::read(&self.key) {
            Ok(key) => key,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut key = vec![0u8; 32];
                getrandom::fill(&mut key).map_err(|error| failed("could not make a key", error))?;
                write_privately(&self.key, &key)?;
                key
            }
            Err(error) => return Err(failed("could not read the key that seals secrets", error)),
        };
        ChaCha20Poly1305::new_from_slice(&key)
            .map_err(|_| SecretsError("the key that seals secrets is not a key".into()))
    }

    fn all(&self) -> Result<BTreeMap<String, String>, SecretsError> {
        let written = match fs::read_to_string(&self.sealed) {
            Ok(written) => written,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(error) => return Err(failed("could not read the secrets", error)),
        };
        let sealed: BTreeMap<String, String> = serde_json::from_str(&written)
            .map_err(|error| failed("the secrets file is damaged", error))?;
        let cipher = self.cipher()?;
        let cannot = || {
            SecretsError("could not unseal the secrets: they were sealed with another key".into())
        };
        sealed
            .into_iter()
            .map(|(name, sealed)| {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(sealed)
                    .map_err(|_| cannot())?;
                if bytes.len() < 12 {
                    return Err(cannot());
                }
                let (nonce, sealed) = bytes.split_at(12);
                let nonce = nonce.try_into().map_err(|_| cannot())?;
                let value = cipher.decrypt(&nonce, sealed).map_err(|_| cannot())?;
                Ok((name, String::from_utf8(value).map_err(|_| cannot())?))
            })
            .collect()
    }

    fn write(&self, all: &BTreeMap<String, String>) -> Result<(), SecretsError> {
        let cipher = self.cipher()?;
        let mut sealed = BTreeMap::new();
        for (name, value) in all {
            let mut nonce = [0u8; 12];
            getrandom::fill(&mut nonce).map_err(|error| failed("could not seal", error))?;
            let mut bytes = nonce.to_vec();
            bytes.extend(
                cipher
                    .encrypt(&nonce.into(), value.as_bytes())
                    .map_err(|error| failed("could not seal", error))?,
            );
            sealed.insert(name, base64::engine::general_purpose::STANDARD.encode(bytes));
        }
        let written =
            serde_json::to_vec_pretty(&sealed).map_err(|error| failed("could not write", error))?;
        write_privately(&self.sealed, &written)
    }
}

/// True of the keys Orbite itself runs on (its Provider's, its bot's): the
/// Agent neither lists nor uses those.
/// `text` with every run of at least [`SHORTEST_PART_HIDDEN`] bytes that is
/// also in `secret` replaced by `named`.
fn without_parts_of(text: &str, secret: &str, named: &str) -> String {
    let (text_bytes, secret_bytes) = (text.as_bytes(), secret.as_bytes());
    if secret_bytes.len() < SHORTEST_PART_HIDDEN || text_bytes.len() < SHORTEST_PART_HIDDEN {
        return text.to_owned();
    }
    // Where each run of that length starts in the secret.
    let mut starts: HashMap<&[u8], Vec<usize>> = HashMap::new();
    for (at, run) in secret_bytes.windows(SHORTEST_PART_HIDDEN).enumerate() {
        starts.entry(run).or_default().push(at);
    }
    let mut shown = Vec::with_capacity(text_bytes.len());
    let mut at = 0;
    while at < text_bytes.len() {
        let run = text_bytes.get(at..at + SHORTEST_PART_HIDDEN);
        let longest = run.and_then(|run| starts.get(run)).map(|found| {
            let same_from = |start: &usize| {
                text_bytes[at..]
                    .iter()
                    .zip(&secret_bytes[*start..])
                    .take_while(|(a, b)| a == b)
                    .count()
            };
            found.iter().map(same_from).max().unwrap_or(0)
        });
        match longest {
            // Never cut inside a letter of several bytes.
            Some(length) if text.is_char_boundary(at) && text.is_char_boundary(at + length) => {
                shown.extend_from_slice(named.as_bytes());
                at += length;
            }
            _ => {
                shown.push(text_bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8(shown).unwrap_or_else(|_| text.to_owned())
}

fn is_orbites_own(name: &str) -> bool {
    name.starts_with("ORBITE_")
}

/// Writes `content` to `path`, whole or not at all, readable by this account only.
fn write_privately(path: &Path, content: &[u8]) -> Result<(), SecretsError> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(|error| failed("could not create the folder", error))?;
    }
    // A name of its own for each file: the key and what it seals are
    // written apart.
    let mut being_written = path.as_os_str().to_owned();
    being_written.push(".writing");
    let being_written = std::path::PathBuf::from(being_written);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file =
        options.open(&being_written).map_err(|error| failed("could not write", error))?;
    file.write_all(content).map_err(|error| failed("could not write", error))?;
    drop(file);
    fs::rename(&being_written, path).map_err(|error| failed("could not write", error))
}

/// True when `text` holds what looks like a secret: a key or token of a kind
/// that is recognisable by its shape.
pub(crate) fn holds_a_secret(text: &str) -> bool {
    const MARKED: &[&str] = &[
        "sk-ant-",
        "sk-proj-",
        "sk-or-",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "glpat-",
        "AIza",
        "hf_",
        "gsk_",
    ];
    if text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----") {
        return true;
    }
    let in_a_key = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    text.split(|c: char| !(in_a_key(c) || c == ':')).any(|word| {
        let marked = MARKED.iter().any(|mark| word.starts_with(mark)) && word.len() >= 20;
        // An OpenAI-style key: long, and with digits, which words do not have.
        let keyed = word.strip_prefix("sk-").is_some_and(|rest| {
            rest.len() >= 24
                && rest.chars().all(in_a_key)
                && rest.chars().any(|c| c.is_ascii_digit())
        });
        let cloud = word.len() == 20
            && word.starts_with("AKIA")
            && word.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        // A Telegram bot token: the bot's number, then its key.
        let bot = word.split_once(':').is_some_and(|(number, key)| {
            (8..=10).contains(&number.len())
                && number.chars().all(|c| c.is_ascii_digit())
                && key.len() >= 30
                && key.chars().all(in_a_key)
        });
        marked || keyed || cloud || bot
    })
}
