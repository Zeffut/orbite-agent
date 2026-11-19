//! Secrets the Agent uses without the model ever seeing them: sealed on the
//! Host, given to a command by name, and hidden from whatever goes back to
//! the model.
//!
//! The key that seals them is a file beside them, readable by the Agent's
//! account only. This keeps secrets out of backups, logs and the model; it
//! does not protect them from someone who has that account.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};

/// The shortest value worth hiding: anything shorter would hide ordinary words.
const SHORTEST_HIDDEN: usize = 6;

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
        let mut all = self.all()?;
        all.insert(name.clone(), value.to_owned());
        self.write(&all)?;
        Ok(name)
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
        hidden
    }

    /// The secrets `command` names, each as (name, value), to give it as
    /// environment variables.
    pub(crate) fn named_in(&self, command: &str) -> Vec<(String, String)> {
        let Ok(all) = self.all() else { return Vec::new() };
        all.into_iter().filter(|(name, _)| command.contains(name.as_str())).collect()
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

/// Writes `content` to `path`, whole or not at all, readable by this account only.
fn write_privately(path: &Path, content: &[u8]) -> Result<(), SecretsError> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(|error| failed("could not create the folder", error))?;
    }
    let being_written = path.with_extension("writing");
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
