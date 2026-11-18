use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use orbite::{Agent, Both, Connected, Store, Telegram, Terminal, connect, known_providers};
use tokio::io::{BufReader, stdin, stdout};

const USAGE: &str = "\
Usage: orbite <command>

Commands:
  run    hold the Conversation in Telegram (add --terminal to open it here as well)
  chat   hold the Conversation in this terminal
  providers  list the Providers known by name

Read from the environment:
  ORBITE_PROVIDER        the Provider, by name (default: openai); see `orbite providers`
  ORBITE_API_KEY         API Key of the Provider (not needed for one on this machine)
  ORBITE_MODEL           model to use (required)
  ORBITE_BASE_URL        the Provider's address, when it is not its usual one
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
        Some("chat") => chat().await,
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
    let token = required("ORBITE_TELEGRAM_TOKEN")?;
    let owner: i64 = required("ORBITE_TELEGRAM_OWNER")?
        .parse()
        .map_err(|_| "ORBITE_TELEGRAM_OWNER must be a numeric Telegram account id.")?;
    let mut agent = agent()?;
    let telegram = Telegram::new(token, owner).saving_files_in(home()?.join("workspace/inbox"));
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
fn agent() -> Result<Agent<Connected>, Failure> {
    let name = setting("ORBITE_PROVIDER").unwrap_or("openai".into());
    let model = required("ORBITE_MODEL")?;
    let provider = connect(
        &name,
        setting("ORBITE_API_KEY").as_deref(),
        &model,
        setting("ORBITE_BASE_URL").as_deref(),
    )?;

    let home = home()?;
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
        .keeping_context_under(context)
        .waking_up_every_minutes(wake_ups))
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
