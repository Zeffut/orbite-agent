use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use orbite::{Agent, OpenAiCompatible, Store, Terminal};
use tokio::io::{BufReader, stdin, stdout};

const USAGE: &str = "\
Usage: orbite chat

Talk to the Agent in this terminal. Reads from the environment:
  ORBITE_API_KEY   API Key of the Provider (required)
  ORBITE_MODEL     model to use (required)
  ORBITE_BASE_URL  Provider's API root (default: https://api.openai.com/v1)
  ORBITE_HOME      folder where the Agent keeps its data (default: ~/.orbite)";

#[tokio::main]
async fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        Some("chat") => chat().await,
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

async fn chat() -> ExitCode {
    let (Some(api_key), Some(model)) = (setting("ORBITE_API_KEY"), setting("ORBITE_MODEL")) else {
        eprintln!("ORBITE_API_KEY and ORBITE_MODEL must both be set.\n\n{USAGE}");
        return ExitCode::from(2);
    };
    let base_url = setting("ORBITE_BASE_URL").unwrap_or("https://api.openai.com/v1".into());

    let store = match open_store() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("Could not open the Agent's data: {error}");
            return ExitCode::FAILURE;
        }
    };

    let provider = OpenAiCompatible::new(base_url, api_key, model);
    let mut terminal = Terminal::new(BufReader::new(stdin()), stdout());
    Agent::new(provider, store).converse(&mut terminal).await;
    ExitCode::SUCCESS
}

/// The Store of this installation, in the folder that holds all its data.
fn open_store() -> Result<Store, Box<dyn std::error::Error>> {
    let home = match setting("ORBITE_HOME") {
        Some(home) => PathBuf::from(home),
        None => PathBuf::from(setting("HOME").ok_or("HOME is not set")?).join(".orbite"),
    };
    fs::create_dir_all(&home)?;
    Ok(Store::open(home.join("orbite.db"))?)
}

fn setting(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}
