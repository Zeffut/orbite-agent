use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use orbite::{
    Agent, Both, Config, Connected, Language, PROVIDER_KEY, RELEASE_KEY, Routed, SETTINGS, Secrets,
    Settings, Setup, SetupStep, Store, Systemd, TELEGRAM_TOKEN, Telegram, Terminal, Updater,
    connect, known_providers,
};
use tokio::io::{BufReader, stdin, stdout};

const USAGE: &str = "\
Usage: orbite <command>

Commands:
  run    hold the Conversation in Telegram (add --terminal to open it here as well)
  chat   hold the Conversation in this terminal
  providers  list the Providers known by name
  version    say which version this is
  setup      set Orbite up, or one part again (language, provider, telegram, name)
  settings   list the settings, or change one (settings NAME VALUE)
  export     write everything of the Agent to a file (export FILE), to move it to another Host
  restore    put an export back on a Host where Orbite is not set up yet (restore FILE)
  uninstall  remove Orbite from this machine (--keep-data keeps the Agent's data)
  update     install the latest release now (on or off: let Orbite do it by itself, or not)
  secret     enter (set NAME), list or remove the secrets the Agent uses without seeing them

Read from the environment:
  ORBITE_PROVIDER        the Provider, by name (default: openai); see `orbite providers`
  ORBITE_API_KEY         API Key of the Provider (not needed for one on this machine)
  ORBITE_MODEL           model to use (required)
  ORBITE_BASE_URL        the Provider's address, when it is not its usual one
  ORBITE_LIGHT_MODEL     a lighter model for summaries and wake-ups (optional)
  ORBITE_FALLBACK_PROVIDER, ORBITE_FALLBACK_MODEL, ORBITE_FALLBACK_API_KEY
                         another Provider for when the usual one cannot answer (optional)
  ORBITE_HOME            folder where the Agent keeps its data (default: ~/.orbite)
  ORBITE_CONTEXT_TOKENS  most tokens sent to the model per turn (default: 60000)
  ORBITE_WAKE_UP_MINUTES minutes between the Agent's own wake-ups, 0 for never (default: 240)
  ORBITE_TELEGRAM_TOKEN  token of the Owner's Telegram bot (run)
  ORBITE_TELEGRAM_OWNER  numeric id of the Owner's Telegram account (run)";

type Failure = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> ExitCode {
    let outcome = match env::args().nth(1).as_deref() {
        Some("run") => run().await,
        Some("version" | "--version" | "-V") => {
            println!("orbite {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("chat") => chat().await,
        Some("secret") => secret(),
        Some("update") => update().await,
        Some("settings") => settings(),
        Some("export") => moving(true),
        Some("restore") => moving(false),
        Some("uninstall") => uninstall(),
        Some("setup") => setup().await,
        Some("providers") => {
            for provider in known_providers() {
                println!("{:<11} {}", provider.name, provider.label);
            }
            println!(
                "{:<11} any server speaking OpenAI's protocol (set ORBITE_BASE_URL)",
                "custom"
            );
            Ok(())
        }
        _ => Err(USAGE.into()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("{failure}");
            ExitCode::from(2)
        }
    }
}

async fn run() -> Result<(), Failure> {
    let home = home()?;
    let token = match setting("ORBITE_TELEGRAM_TOKEN") {
        Some(token) => token,
        None => Secrets::kept_in(&home)
            .get(TELEGRAM_TOKEN)?
            .ok_or("No Telegram bot is set up yet. Run `orbite setup`.")?,
    };
    let owner: i64 = match setting("ORBITE_TELEGRAM_OWNER") {
        Some(owner) => owner
            .parse()
            .map_err(|_| "ORBITE_TELEGRAM_OWNER must be a numeric Telegram account id.")?,
        None => Config::load(&home)?
            .telegram_owner
            .ok_or("No Telegram account is linked yet. Run `orbite setup`.")?,
    };
    let mut agent = agent()?;
    let language = Language::of(Config::load(&home)?.language.as_deref());
    let mut telegram = Telegram::new(token, owner)
        .saving_files_in(home.join("workspace/inbox"))
        .speaking(language);
    if let Some(address) = Config::load(&home)?.telegram_address {
        telegram = telegram.at(address);
    }
    if env::args().any(|argument| argument == "--terminal") {
        // The same Conversation, open in this terminal as well.
        let terminal = Terminal::new(BufReader::new(stdin()), stdout());
        agent.converse(&mut Both::new(telegram, terminal)).await;
    } else {
        agent.converse(&mut telegram).await;
    }
    Ok(())
}

async fn chat() -> Result<(), Failure> {
    let mut agent = agent()?;
    agent.converse(&mut Terminal::new(BufReader::new(stdin()), stdout())).await;
    Ok(())
}

/// The Agent of this installation: its Provider and the folder holding its data.
fn agent() -> Result<Agent<Routed<Connected>>, Failure> {
    // What setup kept, unless the environment says otherwise.
    let home = home()?;
    let config = Config::load(&home)?;
    let secrets = Secrets::kept_in(&home);
    let chosen = config.provider.clone();
    let name = setting("ORBITE_PROVIDER")
        .or(chosen.as_ref().map(|chosen| chosen.name.clone()))
        .ok_or("No AI is set up yet. Run `orbite setup`.")?;
    let model = setting("ORBITE_MODEL")
        .or(chosen.as_ref().map(|chosen| chosen.model.clone()))
        .ok_or("No model is chosen yet. Run `orbite setup`.")?;
    let key = match setting("ORBITE_API_KEY") {
        Some(key) => Some(key),
        None => secrets.get(PROVIDER_KEY)?,
    };
    let address = setting("ORBITE_BASE_URL").or(chosen.and_then(|chosen| chosen.address));
    let mut provider =
        Routed::through(&name, connect(&name, key.as_deref(), &model, address.as_deref())?);
    // Another Provider to fall back on when the usual one cannot answer.
    if let Some(other) = setting("ORBITE_FALLBACK_PROVIDER") {
        let fallback = connect(
            &other,
            setting("ORBITE_FALLBACK_API_KEY").as_deref(),
            &required("ORBITE_FALLBACK_MODEL")?,
            setting("ORBITE_FALLBACK_BASE_URL").as_deref(),
        )?;
        provider = provider.or(&other, fallback);
    }
    // A lighter model for summaries and wake-ups, with the usual Provider
    // unless another is named.
    if let Some(light_model) = setting("ORBITE_LIGHT_MODEL").or(config.light_model.clone()) {
        let light = match setting("ORBITE_LIGHT_PROVIDER") {
            Some(other) => {
                let key = setting("ORBITE_LIGHT_API_KEY");
                (other.clone(), connect(&other, key.as_deref(), &light_model, None)?)
            }
            None => {
                (name.clone(), connect(&name, key.as_deref(), &light_model, address.as_deref())?)
            }
        };
        provider = provider.with_light(&format!("{} (light)", light.0), light.1);
    }

    fs::create_dir_all(&home)
        .map_err(|error| format!("Could not create {}: {error}", home.display()))?;
    let store = Store::open(home.join("orbite.db"))
        .map_err(|error| format!("Could not open the Agent's data: {error}"))?;

    let wake_ups: u32 = match setting("ORBITE_WAKE_UP_MINUTES") {
        Some(minutes) => minutes.parse().map_err(|_| "ORBITE_WAKE_UP_MINUTES must be a number.")?,
        None => 240,
    };
    let context: usize = match setting("ORBITE_CONTEXT_TOKENS") {
        Some(tokens) => tokens.parse().map_err(|_| "ORBITE_CONTEXT_TOKENS must be a number.")?,
        None => 60_000,
    };
    Ok(Agent::new(provider, store)
        .working_in(home.join("workspace"))
        .remembering_in(home.join("memory"))
        .learning_in(home.join("skills"))
        .hearing_notices_in(home.join("notices"))
        .extending_in(home.join("extensions"))
        .connecting_servers_in(&home)
        .configured_in(&home)
        .keeping_secrets_in(&home)
        .called(config.agent_name.clone().unwrap_or_else(|| "Orbite".into()))
        .speaking(Language::of(config.language.as_deref()))
        .keeping_context_under(context)
        .waking_up_every_minutes(wake_ups))
}

/// Goes through setup in this terminal: all of what is missing, or one part again.
async fn setup() -> Result<(), Failure> {
    let only = match env::args().nth(2).as_deref() {
        None => None,
        Some("language") => Some(SetupStep::Language),
        Some("provider") => Some(SetupStep::Provider),
        Some("telegram") => Some(SetupStep::Telegram),
        Some("name") => Some(SetupStep::Name),
        Some(_) => return Err("Usage: orbite setup [language|provider|telegram|name]".into()),
    };
    let typed = std::io::BufReader::new(std::io::stdin());
    let mut setup = Setup::in_home(home()?, typed, std::io::stdout());
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        setup = setup.reading_secrets_with(|question| rpassword::prompt_password(question));
    }
    Ok(setup.run(only).await?)
}

/// Enters, lists or removes the secrets the Agent can use without seeing them.
fn secret() -> Result<(), Failure> {
    const HOW: &str =
        "Usage: orbite secret set NAME | orbite secret list | orbite secret remove NAME";
    let arguments: Vec<String> = env::args().skip(2).collect();
    let secrets = Secrets::kept_in(home()?);
    match arguments.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["set", name] => {
            // Typed without being shown; read as a line when it is piped in.
            let value = if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                rpassword::prompt_password(format!(
                    "Value of {} (not shown): ",
                    name.to_uppercase()
                ))?
            } else {
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                line
            };
            let value = value.trim_end_matches(['\n', '\r']);
            if value.is_empty() {
                return Err("The value is empty: nothing was kept.".into());
            }
            let name = secrets.set(name, value)?;
            println!("Kept as {name}. The agent can use it by that name and never sees its value.");
        }
        ["list"] => {
            for name in secrets.names()? {
                println!("{name}");
            }
        }
        ["remove", name] => {
            if !secrets.remove(name)? {
                return Err(format!("There is no secret called {}.", name.to_uppercase()).into());
            }
            println!("Removed {}.", name.to_uppercase());
        }
        _ => return Err(HOW.into()),
    }
    Ok(())
}

/// Writes everything of the Agent to a file, or puts such a file back.
fn moving(out: bool) -> Result<(), Failure> {
    let home = home()?;
    let Some(file) = env::args().nth(2).map(PathBuf::from) else {
        return Err("Usage: orbite export FILE | orbite restore FILE".into());
    };
    if out {
        orbite::export(&home, &file)?;
        println!(
            "Everything of the Agent is in {}. Keep that file to yourself: it holds its secrets and the \
             key that opens them.",
            file.display()
        );
    } else {
        orbite::restore(&file, &home)?;
        println!(
            "The Agent is back, in {}. Start it with: systemctl restart orbite",
            home.display()
        );
    }
    Ok(())
}

/// Removes Orbite from this machine: the service, the account, the program
/// and, unless asked to keep it, the Agent's data.
fn uninstall() -> Result<(), Failure> {
    let arguments: Vec<String> = env::args().skip(2).collect();
    let keep = arguments.iter().any(|argument| argument == "--keep-data");
    let root = setting("ORBITE_ROOT").unwrap_or_default();
    let data = PathBuf::from(format!("{root}/var/lib/orbite"));
    if !arguments.iter().any(|argument| argument == "--yes") {
        let losing = if keep {
            "The Agent's data is kept."
        } else {
            "The Agent's data is removed with it: its \
             Conversation, Memory, Skills, Extensions and secrets."
        };
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            return Err(
                "Nothing was removed. To remove Orbite without being asked, add --yes.".into()
            );
        }
        println!("This removes Orbite from this machine. {losing}\nType yes to go on:");
        let mut typed = String::new();
        std::io::stdin().read_line(&mut typed)?;
        if typed.trim() != "yes" {
            return Err("Nothing was removed.".into());
        }
    }
    let systemctl = |arguments: &[&str]| {
        let _ = std::process::Command::new("systemctl").args(arguments).status();
    };
    systemctl(&["disable", "--now", "orbite"]);
    systemctl(&["disable", "--now", "orbite-update.timer"]);
    for file in [
        "/etc/systemd/system/orbite.service",
        "/etc/systemd/system/orbite-update.service",
        "/etc/systemd/system/orbite-update.timer",
        "/usr/local/bin/orbite.previous",
        "/usr/local/bin/orbite",
    ] {
        match fs::remove_file(format!("{root}{file}")) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(
                    format!("{file} could not be removed: {error}. Run this as root.").into()
                );
            }
            _ => {}
        }
    }
    systemctl(&["daemon-reload"]);
    if keep {
        println!("Orbite is removed. The Agent's data is kept in {}.", data.display());
        // The account goes, the data stays for whoever reads it next.
        let _ = std::process::Command::new("userdel").arg("orbite").status();
        return Ok(());
    }
    if data.exists() {
        fs::remove_dir_all(&data)?;
    }
    let _ = std::process::Command::new("userdel").arg("orbite").status();
    println!("Orbite is removed, with the Agent's data.");
    Ok(())
}

/// Lists the settings, or changes one: the same as /settings in the Conversation.
fn settings() -> Result<(), Failure> {
    let home = home()?;
    fs::create_dir_all(&home)?;
    let store = Store::open(home.join("orbite.db"))?;
    let usual =
        setting("ORBITE_WAKE_UP_MINUTES").and_then(|minutes| minutes.parse().ok()).unwrap_or(240);
    let settings = Settings::of(&store, Some(&home)).waking_up_by_default_every(usual);
    let arguments: Vec<String> = env::args().skip(2).collect();
    match arguments.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => {
            for key in SETTINGS {
                println!("{key} = {}", settings.get(key)?);
            }
        }
        [key] => {
            println!("{key} = {}   ({})", settings.get(key)?, Settings::choices(key).join(", "))
        }
        [key, value] => {
            settings.set(key, value)?;
            println!("{key} = {}", settings.get(key)?);
        }
        _ => {
            return Err(
                "Usage: orbite settings | orbite settings NAME | orbite settings NAME VALUE".into(),
            );
        }
    }
    Ok(())
}

/// Installs the latest release, or says whether Orbite may do so by itself.
async fn update() -> Result<(), Failure> {
    let home = home()?;
    match env::args().nth(2).as_deref() {
        None => {}
        Some(choice @ ("on" | "off")) => {
            let mut config = Config::load(&home)?;
            config.automatic_updates = (choice == "off").then_some(false);
            config.save(&home)?;
            println!("Automatic updates are {choice}.");
            return Ok(());
        }
        Some(_) => return Err("Usage: orbite update | orbite update on | orbite update off".into()),
    }
    // As for the installer: another place to take releases from, another
    // key to trust and another root folder are for trying this out.
    let release = setting("ORBITE_RELEASE_URL").unwrap_or_else(|| {
        "https://github.com/Zeffut/orbite-agent/releases/latest/download".into()
    });
    let key = match setting("ORBITE_SIGNING_KEY") {
        Some(written) => {
            public_key(&written).ok_or("ORBITE_SIGNING_KEY is not an ed25519 public key.")?
        }
        None => RELEASE_KEY,
    };
    let program = format!("{}/usr/local/bin/orbite", setting("ORBITE_ROOT").unwrap_or_default());
    let done = Updater::new(env!("CARGO_PKG_VERSION"), release, key)
        .installed_at(program)
        .telling_the_owner_in(home)
        .run(&Systemd)
        .await?;
    println!("{done}");
    Ok(())
}

/// The key in a public key file as openssl writes one.
fn public_key(written: &str) -> Option<[u8; 32]> {
    use base64::Engine;
    let inside: String = written.lines().filter(|line| !line.starts_with("-----")).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(inside.trim()).ok()?;
    bytes.get(bytes.len().checked_sub(32)?..)?.try_into().ok()
}

/// The folder that holds everything of this installation.
fn home() -> Result<PathBuf, Failure> {
    Ok(match setting("ORBITE_HOME") {
        Some(home) => PathBuf::from(home),
        None => PathBuf::from(required("HOME")?).join(".orbite"),
    })
}

fn required(name: &str) -> Result<String, Failure> {
    setting(name).ok_or_else(|| format!("{name} must be set.\n\n{USAGE}").into())
}

fn setting(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}
