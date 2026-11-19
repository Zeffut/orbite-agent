use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use orbite::{
    Agent, Both, Config, Connected, Language, PROVIDER_KEY, Routed, Secrets, Setup, SetupStep,
    Store, TELEGRAM_TOKEN, Telegram, Terminal, connect, known_providers,
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
    let telegram = Telegram::new(token, owner)
        .saving_files_in(home.join("workspace/inbox"))
        .speaking(language);
    if env::args().any(|argument| argument == "--terminal") {
        // The same Conversation, open in this terminal as well.
        let terminal = Terminal::new(BufReader::new(stdin()), stdout());
        agent.converse(&mut Both::new(telegram, terminal)).await;
    } else {
        let mut telegram = telegram;
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
