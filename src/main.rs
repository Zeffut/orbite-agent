use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use orbite::{Agent, OpenAiCompatible, Store, Telegram, Terminal};
use tokio::io::{BufReader, stdin, stdout};

const USAGE: &str = "\
Usage: orbite <command>

Commands:
  run    hold the Conversation in Telegram
  chat   hold the Conversation in this terminal

Read from the environment:
  ORBITE_API_KEY         API Key of the Provider (required)
  ORBITE_MODEL           model to use (required)
  ORBITE_BASE_URL        Provider's API root (default: https://api.openai.com/v1)
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
    let mut telegram = Telegram::new(token, owner).saving_files_in(home()?.join("workspace/inbox"));
    agent.converse(&mut telegram).await;
    Ok(())
}

async fn chat() -> Result<(), Failure> {
    let mut agent = agent()?;
    agent.converse(&mut Terminal::new(BufReader::new(stdin()), stdout())).await;
    Ok(())
}

/// The Agent of this installation: its Provider and the folder holding its data.
fn agent() -> Result<Agent<OpenAiCompatible>, Failure> {
    let api_key = required("ORBITE_API_KEY")?;
    let model = required("ORBITE_MODEL")?;
    let base_url = setting("ORBITE_BASE_URL").unwrap_or("https://api.openai.com/v1".into());

    let home = home()?;
    fs::create_dir_all(&home)
        .map_err(|error| format!("Could not create {}: {error}", home.display()))?;
    let store = Store::open(home.join("orbite.db"))
        .map_err(|error| format!("Could not open the Agent's data: {error}"))?;

    let provider = OpenAiCompatible::new(base_url, api_key, model);
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
