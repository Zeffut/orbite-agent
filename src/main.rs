use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use orbite::{
    Agent, Both, Config, Connected, Language, PROVIDER_KEY, RELEASE_KEY, Routed, SETTINGS, Secrets,
    Settings, Setup, SetupStep, Store, Systemd, TELEGRAM_TOKEN, Telegram, Terminal, Updater,
    connect_in, known_providers,
};
use tokio::io::{BufReader, stdin, stdout};

const USAGE: &str = "\
Usage: orbite <command>

Commands:
  run    hold the Conversation in Telegram (add --terminal to open it here as well)
  chat   hold the Conversation in this terminal
  providers  list the Providers set up, in order; add one, or change them (add, first NAME, remove NAME, limit NAME TOKENS)
  version    say which version this is
  setup      set Orbite up, or one part again (language, provider, telegram, name)
  computer   check that the agent's own computer works on this machine (computer check)
  tryout     try the chosen model (or: tryout MODEL) out on a few real tasks and say what to expect of it
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
    // Everything but what changes the machine itself is done as the Agent's
    // own account, even when asked as root.
    let command = env::args().nth(1).unwrap_or_default();
    // (`update on` and `update off` only write the Agent's settings.)
    let changes_the_machine = match command.as_str() {
        "update" => env::args().nth(2).is_none(),
        "uninstall" | "export" | "restore" | "version" | "providers" => true,
        _ => false,
    };
    if !changes_the_machine && let Err(failure) = become_the_agents_account() {
        eprintln!("{failure}");
        return ExitCode::from(2);
    }
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
        Some("tryout") => tryout().await,
        Some("computer") => computer(),
        Some("export") => moving(true),
        Some("restore") => moving(false),
        Some("uninstall") => uninstall(),
        Some("setup") => setup().await,
        Some("providers") => providers().await,
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
        until_asked_to_stop(agent.converse(&mut Both::new(telegram, terminal))).await;
        agent.shut_down();
    } else {
        let mut agent = agent.restarted_by_its_service();
        until_asked_to_stop(agent.converse(&mut telegram)).await;
        agent.shut_down();
    }
    Ok(())
}

async fn chat() -> Result<(), Failure> {
    let mut agent = agent()?;
    until_asked_to_stop(agent.converse(&mut Terminal::new(BufReader::new(stdin()), stdout())))
        .await;
    agent.shut_down();
    Ok(())
}

/// Holds the Conversation until it ends, or until Orbite is asked to stop,
/// as its service does or as Ctrl-C does in a terminal.
async fn until_asked_to_stop(conversing: impl std::future::Future<Output = ()>) {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut stop), Ok(mut interrupt), Ok(mut hung_up)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
        signal(SignalKind::hangup()),
    ) else {
        return conversing.await;
    };
    tokio::select! {
        () = conversing => {}
        _ = stop.recv() => {}
        _ = interrupt.recv() => {}
        // The terminal it ran in was closed.
        _ = hung_up.recv() => {}
    }
}

/// The Provider this installation was set up with, running `model` when one
/// is given rather than the model chosen.
fn usual_provider(model: Option<String>) -> Result<Connected, Failure> {
    let home = home()?;
    let chosen = Config::load(&home)?.provider;
    let name = setting("ORBITE_PROVIDER")
        .or(chosen.as_ref().map(|chosen| chosen.name.clone()))
        .ok_or("No AI is set up yet. Run `orbite setup`.")?;
    let model = model
        .or(setting("ORBITE_MODEL"))
        .or(chosen.as_ref().map(|chosen| chosen.model.clone()))
        .ok_or("No model is chosen yet. Run `orbite setup`.")?;
    let key = match setting("ORBITE_API_KEY") {
        Some(key) => Some(key),
        None => {
            let named = chosen.as_ref().and_then(|chosen| chosen.key.as_deref());
            Secrets::kept_in(&home).get(named.unwrap_or(PROVIDER_KEY))?
        }
    };
    let address = setting("ORBITE_BASE_URL").or(chosen.and_then(|chosen| chosen.address));
    Ok(connect_in(&home, &name, key.as_deref(), &model, address.as_deref())?)
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
    // Each Provider has its key: its own, or the one of the first set up.
    let key_of = |choice: Option<&orbite::ProviderChoice>| -> Result<Option<String>, Failure> {
        let named = choice.and_then(|choice| choice.key.as_deref()).unwrap_or(PROVIDER_KEY);
        Ok(secrets.get(named)?)
    };
    // A key given in the environment is for an Agent with one Provider:
    // with several, it would go to whichever comes first.
    let key = match setting("ORBITE_API_KEY").filter(|_| config.fallbacks.is_empty()) {
        Some(key) => Some(key),
        None => key_of(chosen.as_ref())?,
    };
    let address =
        setting("ORBITE_BASE_URL").or(chosen.as_ref().and_then(|chosen| chosen.address.clone()));
    let mut provider = Routed::through(
        &name,
        connect_in(&home, &name, key.as_deref(), &model, address.as_deref())?,
    );
    if let Some(limit) = chosen.as_ref().and_then(|chosen| chosen.daily_tokens) {
        provider = provider.limited_to(limit);
    }
    // Those that take over, in the order their Owner put them.
    for other in &config.fallbacks {
        let key = key_of(Some(other))?;
        let connected =
            connect_in(&home, &other.name, key.as_deref(), &other.model, other.address.as_deref())?;
        provider = provider.or(&other.name, connected);
        if let Some(limit) = other.daily_tokens {
            provider = provider.limited_to(limit);
        }
    }
    // Another Provider to fall back on when the usual one cannot answer.
    if let Some(other) = setting("ORBITE_FALLBACK_PROVIDER") {
        let fallback = connect_in(
            &home,
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
                (other.clone(), connect_in(&home, &other, key.as_deref(), &light_model, None)?)
            }
            None => (
                name.clone(),
                connect_in(&home, &name, key.as_deref(), &light_model, address.as_deref())?,
            ),
        };
        provider = provider.with_light(&format!("{} (light)", light.0), light.1);
    }

    fs::create_dir_all(&home)
        .map_err(|error| format!("Could not create {}: {error}", home.display()))?;
    let store = Store::open(home.join("orbite.db"))
        .map_err(|error| format!("Could not open the Agent's data: {error}"))?;
    // What was used earlier today counts towards each Provider's limit.
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    for (by, tokens) in store.tokens_used_on(&today).unwrap_or_default() {
        provider = provider.having_used(&by, tokens);
    }

    let wake_ups: u32 = match setting("ORBITE_WAKE_UP_MINUTES") {
        Some(minutes) => minutes.parse().map_err(|_| "ORBITE_WAKE_UP_MINUTES must be a number.")?,
        None => 240,
    };
    let context: usize = match setting("ORBITE_CONTEXT_TOKENS") {
        Some(tokens) => tokens.parse().map_err(|_| "ORBITE_CONTEXT_TOKENS must be a number.")?,
        None => 60_000,
    };
    let mut agent = Agent::new(provider, store);
    // What the Owner says aloud is written down by the first of their
    // Providers that offers it and has a key.
    for listening in config.providers() {
        let Some(model) = orbite::transcription_model(&listening.name) else { continue };
        let Some(key) = key_of(Some(&listening))? else { continue };
        let root = listening.address.clone().or_else(|| {
            let known = known_providers().iter().find(|known| known.name == listening.name)?;
            Some(known.api_root.to_owned())
        });
        if let Some(root) = root {
            let model = setting("ORBITE_TRANSCRIPTION_MODEL").unwrap_or_else(|| model.to_owned());
            agent = agent.transcribing_with(orbite::Transcriber::new(root, key, model));
            break;
        }
    }
    // The Agent has a Computer where the machine has what one is made of.
    if orbite::Computer::can_start() {
        agent = agent.using_a_computer_in(home.join("computer"));
    }
    Ok(agent
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

/// Adds a Provider to those set up, or changes their order, limits and number.
async fn providers() -> Result<(), Failure> {
    let asked: Vec<String> = env::args().skip(2).collect();
    let asked: Vec<&str> = asked.iter().map(String::as_str).collect();
    if asked.is_empty() {
        let home = home()?;
        let language = Language::of(Config::load(&home)?.language.as_deref());
        println!("{}\n", orbite::providers_shown(&home, language, &[]));
        println!(
            "Change them: orbite providers add | first NAME | remove NAME | limit NAME TOKENS\n\nKnown by name:"
        );
        for provider in known_providers() {
            println!("{:<11} {}", provider.name, provider.label);
        }
        println!("{:<11} any server speaking OpenAI's protocol", "custom");
        return Ok(());
    }
    if asked == ["add"] {
        let typed = std::io::BufReader::new(std::io::stdin());
        let mut setup = Setup::in_home(home()?, typed, std::io::stdout());
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            setup = setup.reading_secrets_with(|question| rpassword::prompt_password(question));
        }
        setup.run(Some(SetupStep::AnotherProvider)).await?;
    } else {
        println!("{}", orbite::change_providers(&home()?, &asked).await?);
    }
    println!(
        "It applies at the next start of Orbite (as a service: sudo systemctl restart orbite)."
    );
    Ok(())
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
    // The data first: were it to fail, the program is still there to try again.
    if !keep && data.exists() {
        fs::remove_dir_all(&data)?;
    }
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
        // The account goes and the data stays, given to root first: left to
        // the account's number, a later account given that number would
        // read the Agent's secrets.
        let _ = std::process::Command::new("chown").args(["-R", "root:root"]).arg(&data).status();
        let _ = std::process::Command::new("userdel").arg("orbite").status();
        return Ok(());
    }
    let _ = std::process::Command::new("userdel").arg("orbite").status();
    println!("Orbite is removed, with the Agent's data.");
    println!(
        "The packages installed for the agent's computer (Firefox, xvfb, xdotool, fonts) were left: remove \
         them with apt if nothing else uses them."
    );
    Ok(())
}

/// Starts the Agent's Computer, opens an empty page in its browser, pictures
/// the screen and says whether that worked.
fn computer() -> Result<(), Failure> {
    if env::args().nth(2).as_deref() != Some("check") {
        return Err("Usage: orbite computer check".into());
    }
    let folder = home()?.join("computer");
    // A profile of its own: the Agent's sign-ins are not touched by a check.
    let profile = folder.join("check-profile");
    let failed = |what: &str, error: std::io::Error| -> Failure {
        format!(
            "The agent's computer does not work on this machine: {what} ({error}). It needs Xvfb, xdotool \
             and a browser; running the installer again sets them up."
        )
        .into()
    };
    let mut computer = orbite::Computer::start(&profile, 1280, 800)
        .map_err(|error| failed("it did not start", error))?;
    computer.open("about:blank").map_err(|error| failed("its browser did not open", error))?;
    // A browser shows itself within a few seconds: its window is not the
    // plain black of an empty screen.
    let empty = computer
        .screen()
        .picture()
        .map_err(|error| failed("its screen could not be pictured", error))?;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(40);
    let mut seen = empty.clone();
    while seen == empty && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(500));
        seen = computer
            .screen()
            .picture()
            .map_err(|error| failed("its screen could not be pictured", error))?;
    }
    computer.close_browser();
    drop(computer);
    let _ = fs::remove_dir_all(&profile);
    fs::write(folder.join("check.png"), &seen)?;
    if seen == empty {
        return Err(failed(
            "its browser showed nothing",
            std::io::Error::other("the screen stayed empty"),
        ));
    }
    println!(
        "The agent's computer works. What it saw is in {}.",
        folder.join("check.png").display()
    );
    Ok(())
}

/// Tries the chosen model out and says what an Owner can expect of it.
async fn tryout() -> Result<(), Failure> {
    let home = home()?;
    let language = Language::of(Config::load(&home)?.language.as_deref());
    let provider = usual_provider(env::args().nth(2))?;
    println!(
        "{}",
        language.say(
            "Trying the model out on a few real tasks...",
            "Essai du modèle sur quelques vraies tâches..."
        )
    );
    let verdict = orbite::try_out(&provider).await.map_err(|error| match error {
        orbite::ProviderError::Rejected(why) => format!("The Provider refused the key: {why}"),
        orbite::ProviderError::Unreachable(why) => {
            format!("The Provider could not be reached: {why}")
        }
        orbite::ProviderError::Busy { reason, .. } => {
            format!("The Provider is too busy to try the model now: {reason}")
        }
        orbite::ProviderError::Declined(why) | orbite::ProviderError::Failed(why) => {
            format!("The Provider failed on the first task: {why}")
        }
    })?;
    println!("{}", verdict.told(language));
    // Kept, to be shown wherever this model is offered.
    let chosen = Config::load(&home)?.provider;
    let name = setting("ORBITE_PROVIDER").or(chosen.as_ref().map(|chosen| chosen.name.clone()));
    let model =
        env::args().nth(2).or(setting("ORBITE_MODEL")).or(chosen.map(|chosen| chosen.model));
    if let (Some(name), Some(model)) = (name, model) {
        orbite::keep_rating(&home, &name, &model, &verdict)?;
    }
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

/// The folder that holds everything of this installation: the one named
/// by ORBITE_HOME, else the one of an Orbite installed on this machine when
/// it can be read, else `.orbite` in the home of whoever runs this.
fn home() -> Result<PathBuf, Failure> {
    if let Some(home) = setting("ORBITE_HOME") {
        return Ok(PathBuf::from(home));
    }
    let installed =
        PathBuf::from(format!("{}/var/lib/orbite", setting("ORBITE_ROOT").unwrap_or_default()));
    if fs::read_dir(&installed).is_ok() {
        return Ok(installed);
    }
    Ok(PathBuf::from(required("HOME")?).join(".orbite"))
}

/// Run as root on an installed machine, goes on as the account the Agent's
/// folder belongs to: what is written there must stay the Agent's to read.
fn become_the_agents_account() -> Result<(), Failure> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: plain system calls, made before anything else is started.
        if unsafe { libc::geteuid() } != 0 {
            return Ok(());
        }
        let Ok(folder) = fs::metadata(home()?) else { return Ok(()) };
        if folder.uid() == 0 {
            return Ok(());
        }
        // SAFETY: as above; the group is changed first, while still allowed to.
        let changed = unsafe {
            libc::setgroups(0, std::ptr::null()) == 0
                && libc::setgid(folder.gid()) == 0
                && libc::setuid(folder.uid()) == 0
        };
        if !changed {
            return Err("Could not go on as the account Orbite runs as.".into());
        }
    }
    Ok(())
}

fn required(name: &str) -> Result<String, Failure> {
    setting(name).ok_or_else(|| format!("{name} must be set.\n\n{USAGE}").into())
}

fn setting(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}
