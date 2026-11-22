//! What an installation was set up with, kept in its folder as a file the
//! Owner can read. Nothing secret is in it: keys and tokens are sealed apart.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The settings of an installation. Everything is optional: setup fills it
/// in step by step.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// The language of Orbite's own words, as a two-letter code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The Provider that normally answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderChoice>,
    /// The Providers that take over, in order, when the one before cannot
    /// answer or used up its limit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<ProviderChoice>,
    /// A lighter model of the same Provider, for summaries and Wake-ups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light_model: Option<String>,
    /// The Telegram account the Agent obeys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_owner: Option<i64>,
    /// Where Telegram's bot API is reached, when it is not Telegram's own
    /// servers (a bot API server the Owner runs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_address: Option<String>,
    /// False when the Owner does not want Orbite to update itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_updates: Option<bool>,
    /// How speech is written down, when the Owner chose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcription: Option<Transcription>,
    /// What the Owner called their Agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// Where the web interface is served, when the Owner turned it on:
    /// an address and a port, such as `127.0.0.1:8377`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
    /// The address the Owner's other devices reach the web interface at,
    /// when something serves it to them encrypted: `https://…`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_reached_at: Option<String>,
    /// The account of this machine whose desktop the Agent may be lent, by
    /// its number: the one that installed Orbite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desk_account: Option<u32>,
}

/// What writes down what the Owner says aloud.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Transcription {
    /// `provider` (the first Provider that offers it), `whisper.cpp`,
    /// `server`, `command` or `off`.
    pub engine: String,
    /// The model: a name for whisper.cpp or a server, or a file of the Owner's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Where a local server is reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The Owner's own command, `{file}` standing for the recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The language spoken, when it is not left to be found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// whisper.cpp's program, when it is not `whisper-cli` on the PATH.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
}

/// A Provider and the model to use with it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderChoice {
    /// Its name in the catalogue, or `custom`.
    pub name: String,
    pub model: String,
    /// Its address, when it is not the catalogue's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The name of the secret holding its key, when it has one of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The tokens it may use in a day, read and written together, when its
    /// Owner set a limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_tokens: Option<u64>,
}

impl Config {
    /// The settings kept in `home`; empty when none were kept yet.
    pub fn load(home: impl AsRef<Path>) -> io::Result<Self> {
        match fs::read_to_string(home.as_ref().join("config.json")) {
            Ok(written) => serde_json::from_str(&written).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, home: impl AsRef<Path>) -> io::Result<()> {
        fs::create_dir_all(home.as_ref())?;
        let written = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let being_written = home.as_ref().join("config.writing");
        fs::write(&being_written, written + "\n")?;
        fs::rename(being_written, home.as_ref().join("config.json"))
    }
}
