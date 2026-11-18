use std::env;
use std::process::ExitCode;

use orbite::{Agent, OpenAiCompatible, Terminal};
use tokio::io::{BufReader, stdin, stdout};

const USAGE: &str = "\
Usage: orbite chat

Talk to the Agent in this terminal. Reads from the environment:
  ORBITE_API_KEY   API Key of the Provider (required)
  ORBITE_MODEL     model to use (required)
  ORBITE_BASE_URL  Provider's API root (default: https://api.openai.com/v1)";

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

    let provider = OpenAiCompatible::new(base_url, api_key, model);
    let mut terminal = Terminal::new(BufReader::new(stdin()), stdout());
    Agent::new(provider).converse(&mut terminal).await;
    ExitCode::SUCCESS
}

fn setting(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}
