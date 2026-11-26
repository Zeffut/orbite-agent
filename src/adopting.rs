//! An installation of before becomes the first Agent of a server (ADR
//! 0022): everything of the Agent goes into that Agent's own container,
//! and what is the Owner's rather than an Agent's stays with the server.

use std::io;
use std::path::{Path, PathBuf};

use crate::server::{Docker, Held};
use crate::{Config, Secrets};

/// What the server keeps of an installation, by the names of its files:
/// who is let in, how it is reached, who the Agents think with. All the
/// rest is the Agent's, and leaves with it.
const THE_SERVERS: &[&str] = &[
    "config.json",
    "secrets.key",
    "secrets.lock",
    "secrets.sealed",
    "interface.db",
    "interface.db-wal",
    "interface.db-shm",
    "agents.json",
    "door",
    "chatgpt.renewing",
    "terminal.device",
];

/// The secrets that are the server's: the keys and sign-ins of the Owner's
/// AI providers. An Agent held by a server has none of them.
fn secrets_of_the_server(config: &Config) -> Vec<String> {
    let mut names = vec![crate::PROVIDER_KEY.to_owned(), crate::chatgpt::KEPT_AS.to_owned()];
    for choice in config.provider.iter().chain(&config.fallbacks) {
        names.extend(choice.key.clone());
    }
    names.iter().map(|name| name.trim().to_uppercase()).collect()
}

/// True for a secret that is the server's: one named above, or the key of
/// any AI provider, set up still or not.
fn is_the_servers(name: &str, named: &[String]) -> bool {
    name.starts_with("ORBITE_KEY_") || named.iter().any(|kept| kept == name)
}

fn told(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// Makes of `folder`, a copy of an installation, what the Agent keeps in
/// its container: no key to an AI provider, no device let in, nothing of
/// how the server is reached.
pub fn as_the_agent(folder: &Path) -> io::Result<()> {
    let mut config = Config::load(folder)?;
    let secrets = Secrets::kept_in(folder);
    let the_servers = secrets_of_the_server(&config);
    for name in secrets.names().map_err(told)? {
        if is_the_servers(&name, &the_servers) {
            secrets.remove(&name).map_err(told)?;
        }
    }
    config.provider = None;
    config.fallbacks = Vec::new();
    config.light_model = None;
    config.interface = None;
    config.interface_reached_at = None;
    config.interface_public = None;
    // The program an Agent runs is its image's: the server updates it.
    config.automatic_updates = Some(false);
    config.save(folder)?;
    // Devices are let in by the server, which says which one is asking.
    let devices = crate::interface::kept_in(folder)?;
    devices
        .execute_batch("DELETE FROM devices; DELETE FROM codes; DELETE FROM wrong_codes;")
        .map_err(told)?;
    for gone in ["agents.json", "terminal.device", "chatgpt.renewing"] {
        let _ = std::fs::remove_file(folder.join(gone));
    }
    // A copy of an installation lying there holds everything of it, the
    // keys to its AI providers included: not the Agent's to take along.
    for entry in std::fs::read_dir(folder)?.flatten() {
        if entry.path().extension().is_some_and(|ending| ending == "orbite") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let _ = std::fs::remove_dir_all(folder.join("door"));
    Ok(())
}

/// Leaves in `home` only what is the server's, once its Agent has gone to
/// a container with everything of its own.
pub fn as_the_server(home: &Path) -> io::Result<()> {
    let mut config = Config::load(home)?;
    let kept = secrets_of_the_server(&config);
    let secrets = Secrets::kept_in(home);
    for name in secrets.names().map_err(told)? {
        if !is_the_servers(&name, &kept) {
            secrets.remove(&name).map_err(told)?;
        }
    }
    config.agent_name = None;
    config.agent_shape = None;
    config.agent_colour = None;
    config.telegram_owner = None;
    config.telegram_address = None;
    config.transcription = None;
    config.save(home)?;
    // What was said is the Agent's: the server keeps who is let in.
    let devices = crate::interface::kept_in(home)?;
    // Gone from the file itself, and from what is kept beside it: not only
    // marked as gone.
    devices
        .execute_batch("DELETE FROM shown; VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(told)?;
    drop(devices);
    for entry in std::fs::read_dir(home)? {
        let entry = entry?;
        if THE_SERVERS.iter().any(|kept| entry.file_name() == *kept) {
            continue;
        }
        let gone = entry.path();
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(gone)?;
        } else {
            std::fs::remove_file(gone)?;
        }
    }
    Ok(())
}

/// Puts everything of the installation kept in `home` into the volume of
/// a new Agent, the first of the server `home` then becomes. `home` itself
/// is left as it is: it is cleared with `as_the_server` once the Agent
/// answers from its container. Everything is first written to `backup`,
/// from which the installation can be put back as it was.
pub fn adopt(home: &Path, docker: &Docker, backup: &Path) -> Result<Held, String> {
    fn said(what: &'static str) -> impl Fn(io::Error) -> String {
        move |error| format!("{what}: {error}")
    }
    if home.join("agents.json").exists() {
        return Err("this installation already is a server".into());
    }
    let config =
        Config::load(home).map_err(said("the installation's settings could not be read"))?;
    if config.interface_public.is_some() {
        return Err(
            "this installation is reached from the Internet under a name of its own, which \
                    a server does not serve yet: it is left as it is"
                .into(),
        );
    }
    if let Err(error) = crate::export(home, backup) {
        // What was written of it is no copy of anything.
        let _ = std::fs::remove_file(backup);
        return Err(format!("the installation could not be saved first: {error}"));
    }
    // A copy to work on, where nobody else can look: beside the folder
    // itself, on the same disk.
    let beside = home.parent().unwrap_or(Path::new("/"));
    let staged = tempfile::Builder::new()
        .prefix(".orbite-adopting-")
        .tempdir_in(beside)
        .map_err(said("no room could be made to prepare the agent"))?;
    let folder: PathBuf = staged.path().join("agent");
    crate::restore(backup, &folder)
        .map_err(said("the saved installation could not be read back"))?;
    as_the_agent(&folder).map_err(said("the agent's own could not be told from the server's"))?;
    let name = config.agent_name.clone().unwrap_or_else(|| "Orbite".to_owned());
    let held = Held::new(&name, &[]).map_err(said("nothing random could be made"))?;
    if let Err(why) = docker.fill(&held.id, &folder) {
        let _ = crate::server::Engine::remove(docker, &held.id);
        return Err(format!("the agent's container could not be filled: {why}"));
    }
    Ok(held)
}

/// Writes down the Agents of the server kept in `home`.
pub fn keep(home: &Path, agents: &[Held]) -> io::Result<()> {
    use std::io::Write;
    let mut opening = std::fs::OpenOptions::new();
    opening.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opening.mode(0o600);
    }
    let mut file = opening.open(home.join("agents.json"))?;
    file.write_all((serde_json::to_string_pretty(agents)? + "\n").as_bytes())?;
    file.sync_all()
}

impl Held {
    /// What the server shows this Agent to be known by it.
    pub fn token(&self) -> &str {
        &self.token
    }
}
