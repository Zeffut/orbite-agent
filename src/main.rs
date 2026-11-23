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
  transcription  how voice messages are written down: a provider, whisper.cpp on this machine, a local server, a command of yours, or off
  connections  every connection: the AIs and the connectors (add NAME ADDRESS, remove NAME)
  providers  list the Providers set up, in order; add one, or change them (add, first NAME, remove NAME, limit NAME TOKENS)
  version    say which version this is
  setup      set Orbite up, or one part again (language, provider, telegram, name)
  computer   check that the agent's own computer works on this machine (computer check)
  tryout     try the chosen model (or: tryout MODEL) out on a few real tasks and say what to expect of it
  settings   list the settings, or change one (settings NAME VALUE)
  export     write everything of the Agent to a file (export FILE), to move it to another Host
  restore    put an export back on a Host where Orbite is not set up yet (restore FILE)
  uninstall  remove Orbite from this machine (--keep-data keeps the Agent's data, --keep-computer what was installed for its computer)
  update     install the latest release now (on or off: let Orbite do it by itself, or not)
  secret     enter (set NAME), list or remove the secrets the Agent uses without seeing them
  interface  the web interface: say where it is served, turn it on (on [ADDRESS]) or off
  pair       show a code that lets one more device into the web interface
  devices    list the devices let into the web interface (remove NAME turns one away)

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

fn main() -> ExitCode {
    // The service is let listen on the ports the Internet expects, and
    // nothing it starts is: the right is kept from passing on, before any
    // other thread exists to pass it.
    // SAFETY: a plain system call, on this process alone.
    unsafe { libc::prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0) };
    match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(asked()),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

/// Does what was asked on the command line.
async fn asked() -> ExitCode {
    // Everything but what changes the machine itself is done as the Agent's
    // own account, even when asked as root.
    let command = env::args().nth(1).unwrap_or_default();
    // (`update on` and `update off` only write the Agent's settings.)
    let changes_the_machine = match command.as_str() {
        "update" => env::args().nth(2).is_none(),
        // Which computer the Agent uses is written as root, once.
        "setup" => env::args().nth(2).as_deref() == Some("computer"),
        "uninstall" | "export" | "restore" | "version" | "providers" | "news" => true,
        // Offered from the session of whoever opened the desktop, as them;
        // and whose desktop it may be is written as root, out of the
        // Agent's reach.
        "desk" => true,
        _ => false,
    };
    // What Tailscale is asked takes whoever ran this, root or its operator:
    // asked before going on as the Agent's account.
    let reach = if command == "interface" { reach() } else { None };
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
        Some("news") => {
            for line in news() {
                println!("{line}");
            }
            Ok(())
        }
        Some("secret") => secret(),
        Some("update") => update().await,
        Some("settings") => settings(),
        Some("tryout") => tryout().await,
        Some("computer") => computer(),
        Some("export") => moving(true),
        Some("restore") => moving(false),
        Some("uninstall") => uninstall(),
        Some("setup") if env::args().nth(2).as_deref() == Some("computer") => choose_computer(),
        Some("setup") => setup().await,
        Some("providers") => providers().await,
        Some("connections") => connections(),
        Some("interface") => interface(reach),
        Some("pair") => pair(),
        Some("devices") => devices(),
        Some("desk") => desk(),
        Some("transcription") => {
            let asked: Vec<String> = env::args().skip(2).collect();
            let asked: Vec<&str> = asked.iter().map(String::as_str).collect();
            match home() {
                Ok(home) => orbite::transcription(&home, &asked, &mut std::io::stdout())
                    .await
                    .map_err(Failure::from),
                Err(failure) => Err(failure),
            }
        }
        _ => Err(USAGE.into()),
    };
    let code = match outcome {
        Ok(()) => 0,
        Err(failure) => {
            eprintln!("{failure}");
            2
        }
    };
    // Ended here rather than by returning: a line still being waited for
    // from a terminal would hold the program open until someone typed it.
    // Everything that was running has been stopped and put away by now.
    {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
    }
    std::process::exit(code)
}

async fn run() -> Result<(), Failure> {
    let home = home()?;
    let config = Config::load(&home)?;
    let language = Language::of(config.language.as_deref());
    // Telegram, when a bot was set up and an account linked.
    let token = match setting("ORBITE_TELEGRAM_TOKEN") {
        Some(token) => Some(token),
        None => Secrets::kept_in(&home).get(TELEGRAM_TOKEN)?,
    };
    let owner: Option<i64> = match setting("ORBITE_TELEGRAM_OWNER") {
        Some(owner) => Some(
            owner
                .parse()
                .map_err(|_| "ORBITE_TELEGRAM_OWNER must be a numeric Telegram account id.")?,
        ),
        None => config.telegram_owner,
    };
    let telegram = match (token, owner) {
        (Some(token), Some(owner)) => {
            let telegram = Telegram::new(token, owner)
                .saving_files_in(home.join("workspace/inbox"))
                .speaking(language);
            Some(match &config.telegram_address {
                Some(address) => telegram.at(address),
                None => telegram,
            })
        }
        _ => None,
    };
    // The web interface, when the Owner turned it on.
    let interface = match setting("ORBITE_INTERFACE").or(config.interface.clone()) {
        Some(address) => Some(interface_at(&home, &address, &config).await?),
        None => None,
    };
    // On a Host with an address on the Internet, the door there: opened
    // beside the Conversation, which goes on whether or not it opens.
    let public = config.interface_public.as_deref().and_then(|address| address.parse().ok());
    if let (Some(interface), Some(public)) = (&interface, public) {
        let mut door = orbite::Door::of(&home, public, interface.address());
        // For trying it out against an authority of one's own, on ports
        // that take no privilege.
        if let Some(directory) = setting("ORBITE_ACME_DIRECTORY") {
            door = door.asking(directory, setting("ORBITE_ACME_ROOT").map(PathBuf::from));
        }
        if let Some(ports) = setting("ORBITE_DOOR_PORTS")
            && let Some((plain, encrypted)) = ports.split_once(',')
            && let (Ok(plain), Ok(encrypted)) = (plain.parse(), encrypted.parse())
        {
            let on = |port| std::net::SocketAddr::new(public, port);
            door = door.listening_on(on(plain), on(encrypted));
        }
        let kept = home.join("door");
        tokio::spawn(async move {
            if let Err(error) = door.open().await {
                let trouble = match error.kind() {
                    std::io::ErrorKind::AddrInUse => format!(
                        "something else on this machine already listens on port 80 or 443 \
                         ({error}): they must be left to Orbite"
                    ),
                    _ => format!(
                        "ports 80 and 443 of this machine could not be listened on ({error}): \
                         run the installer again, which lets Orbite's service use them"
                    ),
                };
                eprintln!("The web interface is not reached from the Internet: {trouble}");
                let _ = fs::create_dir_all(&kept);
                let _ = fs::write(kept.join("trouble"), trouble);
            }
        });
    }
    let agent = agent()?;
    let terminal = env::args()
        .any(|argument| argument == "--terminal")
        .then(|| Terminal::new(BufReader::new(stdin()), stdout()));
    // One Conversation, in every place that was set up.
    match (telegram, interface, terminal) {
        (None, None, _) => {
            Err("There is nowhere to talk to your agent yet. Run `orbite setup`, or \
                                turn the web interface on with `orbite interface on`."
                .into())
        }
        (Some(telegram), None, None) => hold(agent, telegram, true).await,
        (None, Some(interface), None) => hold(agent, interface, true).await,
        (Some(telegram), Some(interface), None) => {
            hold(agent, Both::new(telegram, interface), true).await
        }
        (Some(telegram), None, Some(terminal)) => {
            hold(agent, Both::new(telegram, terminal), false).await
        }
        (None, Some(interface), Some(terminal)) => {
            hold(agent, Both::new(interface, terminal), false).await
        }
        (Some(telegram), Some(interface), Some(terminal)) => {
            hold(agent, Both::new(Both::new(telegram, interface), terminal), false).await
        }
    }
}

/// Holds the Conversation in `channel` until Orbite is asked to stop. As a
/// service, Orbite ends when a change takes a restart, to be started again.
async fn hold(
    agent: Agent<Routed<Connected>>,
    mut channel: impl orbite::Channel,
    as_a_service: bool,
) -> Result<(), Failure> {
    let mut agent = if as_a_service { agent.restarted_by_its_service() } else { agent };
    until_asked_to_stop(agent.converse(&mut channel)).await;
    agent.shut_down();
    Ok(())
}

/// The web interface served at `address`, which must be one where nothing
/// travels in the open.
async fn interface_at(
    home: &std::path::Path,
    address: &str,
    config: &Config,
) -> Result<orbite::Interface, Failure> {
    let address: std::net::SocketAddr = address.parse().map_err(|_| {
        format!("{address} is not an address the interface can be served at: write one such as 127.0.0.1:8377.")
    })?;
    if !orbite::is_private(&address) && setting("ORBITE_INTERFACE_IN_THE_OPEN").is_none() {
        return Err(format!(
            "The interface is not served at {address}: it is reached from anywhere, and what travels \
             there is not encrypted yet. Serve it on the machine itself (127.0.0.1) behind something \
             that encrypts, or on an address of your own private network such as Tailscale's."
        )
        .into());
    }
    let language = config.language.as_deref().unwrap_or("en");
    orbite::Interface::serving(home, address, language)
        .await
        .map_err(|error| format!("The interface could not be served at {address}: {error}").into())
}

async fn chat() -> Result<(), Failure> {
    // Where the Agent serves its Conversation, a terminal is one more
    // device in it: the same Conversation as on its page and in its apps.
    let home = home()?;
    let config = Config::load(&home)?;
    let in_a_terminal = std::io::IsTerminal::is_terminal(&std::io::stdin())
        && std::io::IsTerminal::is_terminal(&std::io::stdout())
        && orbite::Welcome::fits_here();
    if in_a_terminal
        && let Some(address) = config.interface.as_deref().and_then(|address| address.parse().ok())
    {
        let language = orbite::Language::of(config.language.as_deref());
        return match orbite::talk(&home, address, language).await {
            Ok(()) => Ok(()),
            Err(error) => Err(format!(
                "Your agent could not be reached at {address}: {error}. Is it running? \
                 (sudo systemctl status orbite)"
            )
            .into()),
        };
    }
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
    // Unless the Owner chose otherwise: the machine itself, or nothing.
    let own_way = config.transcription.clone().filter(|chosen| chosen.engine != "provider");
    if let Some(chosen) = &own_way {
        match orbite::chosen_transcriber(&home, chosen) {
            Ok(Some(transcriber)) => agent = agent.transcribing_with(transcriber),
            Ok(None) => {}
            // The Agent starts all the same, and says why it cannot hear.
            Err(why) => {
                eprintln!("{why}");
                agent = agent.transcribing_with(orbite::Transcriber::unable(why));
            }
        }
    }
    let mut hears = own_way.is_some();
    for listening in config.providers().into_iter().filter(|_| own_way.is_none()) {
        let Some(model) = orbite::transcription_model(&listening.name) else { continue };
        // The first one's key may have been given in the environment.
        let is_first = chosen.as_ref().is_some_and(|first| first.name == listening.name);
        let own = if is_first { key.clone() } else { key_of(Some(&listening))? };
        let Some(key) = own else { continue };
        let root = listening.address.clone().or_else(|| {
            let known = known_providers().iter().find(|known| known.name == listening.name)?;
            Some(known.api_root.to_owned())
        });
        if let Some(root) = root {
            let model = setting("ORBITE_TRANSCRIPTION_MODEL").unwrap_or_else(|| model.to_owned());
            agent = agent.transcribing_with(orbite::Transcriber::new(root, key, model));
            hears = true;
            break;
        }
    }
    // Failing those, the Providers named in the environment: the usual one,
    // then the one to fall back on.
    let named_there = [
        (Some(name.clone()), key.clone(), address.clone()),
        (
            setting("ORBITE_FALLBACK_PROVIDER"),
            setting("ORBITE_FALLBACK_API_KEY"),
            setting("ORBITE_FALLBACK_BASE_URL"),
        ),
    ];
    for (listening, key, address) in named_there {
        if hears {
            break;
        }
        let (Some(listening), Some(key)) = (listening, key) else { continue };
        let Some(model) = orbite::transcription_model(&listening) else { continue };
        let root = address.or_else(|| {
            let known = known_providers().iter().find(|known| known.name == listening)?;
            Some(known.api_root.to_owned())
        });
        if let Some(root) = root {
            let model = setting("ORBITE_TRANSCRIPTION_MODEL").unwrap_or_else(|| model.to_owned());
            agent = agent.transcribing_with(orbite::Transcriber::new(root, key, model));
            hears = true;
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
        .switching_the_interface_with(switch_the_interface)
        // Nobody's desktop is taken where the Owner chose a computer of
        // the Agent's own, when Orbite was installed.
        .lent_this_computer_by(
            if computer_chosen().as_deref() == Some("own") {
                u32::MAX
            } else {
                desk_account().unwrap_or(u32::MAX)
            },
            orbite::DESK_SOCKET,
        )
        .connecting_servers_in(&home)
        .configured_in(&home)
        .keeping_secrets_in(&home)
        .called(config.agent_name.clone().unwrap_or_else(|| "Orbite".into()))
        .speaking(Language::of(config.language.as_deref()))
        .keeping_context_under(context)
        .waking_up_every_minutes(wake_ups))
}

/// Where the web interface is served, or turns it on or off.
/// Has Tailscale, where this machine has it, serve the web interface to
/// the Owner's other devices when it is turned on for this machine alone,
/// or stop when it is turned off. `None` when Tailscale was not asked.
fn reach() -> Option<Result<String, String>> {
    let asked: Vec<String> = env::args().skip(2).collect();
    let tailscale = orbite::tailscale()?;
    match asked.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["on"] => Some(orbite::open_on_tailscale(&tailscale, DEFAULT_INTERFACE.parse().ok()?)),
        // Off, or reached another way: what Tailscale served is closed.
        ["off"] | ["public"] | ["public", _] => {
            let _ = orbite::close_on_tailscale(&tailscale, DEFAULT_INTERFACE.parse().ok()?);
            None
        }
        _ => None,
    }
}

/// Where the web interface is served unless told otherwise: on the machine
/// itself, where nothing is open to anyone.
const DEFAULT_INTERFACE: &str = "127.0.0.1:8377";

fn interface(reach: Option<Result<String, String>>) -> Result<(), Failure> {
    let home = home()?;
    let mut config = Config::load(&home)?;
    let asked: Vec<String> = env::args().skip(2).collect();
    match asked.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => match &config.interface {
            Some(address) => {
                println!(
                    "The web interface is served at {}\nLet a device in: orbite pair",
                    config.interface_reached_at.clone().unwrap_or(format!("http://{address}"))
                );
                // What kept it from being reached from the Internet, if anything.
                if config.interface_public.is_some()
                    && let Ok(trouble) = fs::read_to_string(home.join("door/trouble"))
                {
                    println!("It is not reached yet: {trouble}.");
                }
            }
            None => println!("The web interface is off. Turn it on: orbite interface on"),
        },
        ["on"] | ["on", _] => {
            // On the machine itself unless told otherwise: nothing is open
            // to anyone the Owner did not mean to reach it.
            let address = asked.get(1).map_or(DEFAULT_INTERFACE, String::as_str);
            let parsed: std::net::SocketAddr = address.parse().map_err(|_| {
                format!("{address} is not an address: write one such as 127.0.0.1:8377, or your machine's address on your private network and a port.")
            })?;
            if !orbite::is_private(&parsed) {
                return Err(format!(
                    "{address} is reached from anywhere, and what travels there is not encrypted yet. \
                     Use 127.0.0.1 behind something that encrypts, or an address of your private network."
                )
                .into());
            }
            config.interface = Some(address.to_owned());
            config.interface_reached_at = reach.clone().and_then(Result::ok);
            config.interface_public = None;
            config.save(&home)?;
            let applies = "It applies at the next start of Orbite (as a service: sudo systemctl \
                           restart orbite).\nThen let a device in: orbite pair";
            match reach {
                Some(Ok(reached)) => println!(
                    "The web interface will be reached at {reached}\nfrom the devices of your \
                     Tailscale network, encrypted. Nobody else reaches it.\n{applies}"
                ),
                Some(Err(why)) => println!(
                    "The web interface will be served on this machine only (http://{address}): \
                     Tailscale could not serve it to your other devices.\n{why}.\n{applies}"
                ),
                None => println!("The web interface will be served at http://{address}\n{applies}"),
            }
        }
        // Served encrypted by something of the Owner's own, in front of it.
        ["at", reached] if reached.starts_with("https://") && config.interface.is_some() => {
            let reached = reached.trim_end_matches('/');
            config.interface_reached_at = Some(reached.to_owned());
            config.save(&home)?;
            println!(
                "The web interface is reached at {reached}\nIt applies at the next start of Orbite."
            );
        }
        // On the Host's own address on the Internet, encrypted under a
        // certificate asked for that address.
        ["public"] | ["public", _] => {
            let address = match asked.get(1) {
                Some(given) => given.parse().map_err(|_| format!("{given} is not an address."))?,
                None => orbite::public_address().ok_or(
                    "This machine carries no address of the Internet itself. If it is reached \
                     at one all the same, say which: orbite interface public ADDRESS",
                )?,
            };
            if !orbite::is_of_the_internet(address) {
                return Err(format!(
                    "{address} is not an address the Internet reaches a machine at. On a \
                     private network, use Tailscale: orbite interface on"
                )
                .into());
            }
            let named = match address {
                std::net::IpAddr::V4(address) => address.to_string(),
                std::net::IpAddr::V6(address) => format!("[{address}]"),
            };
            config.interface = Some(DEFAULT_INTERFACE.to_owned());
            config.interface_reached_at = Some(format!("https://{named}"));
            config.interface_public = Some(address.to_string());
            config.save(&home)?;
            println!(
                "The web interface will be reached at https://{named}\nencrypted, under a \
                 certificate Let's Encrypt gives to that address. Ports 80 and 443 of this \
                 machine must be open to the Internet.\nIt applies at the next start of Orbite \
                 (as a service: sudo systemctl restart orbite).\nThen let a device in: orbite pair"
            );
        }
        ["off"] => {
            config.interface = None;
            config.interface_reached_at = None;
            config.interface_public = None;
            config.save(&home)?;
            println!("The web interface is off. It applies at the next start of Orbite.");
        }
        _ => {
            return Err(
                "Usage: orbite interface | interface on [ADDRESS:PORT] | interface public \
                 [ADDRESS] | interface at https://NAME (once it is on) | interface off"
                    .into(),
            );
        }
    }
    Ok(())
}

/// Shows a code that lets one more device into the web interface.
fn pair() -> Result<(), Failure> {
    let home = home()?;
    let config = Config::load(&home)?;
    let Some(address) = config.interface else {
        return Err("The web interface is off. Turn it on first: orbite interface on".into());
    };
    let at = config.interface_reached_at.unwrap_or(format!("http://{address}"));
    let code = orbite::pairing_code(&home)?;
    println!(
        "Open {at} on the device, and type this code there:\n\n    {code}\n\nIt works once, for ten minutes."
    );
    // In a terminal, a code to point a phone at, where a phone can follow it.
    let of_this_machine = at.starts_with("http://127.") || at.starts_with("http://localhost");
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) && !of_this_machine {
        println!(
            "\nOr point your phone's camera at this:\n\n{}",
            orbite::square_code_text(&format!("{at}/#{code}"))
        );
    }
    Ok(())
}

/// Offers this machine's desktop to the Agent, from the graphical session of
/// whoever opened it; or, as root when Orbite is installed, writes down
/// whose desktop may be offered (`desk allow ACCOUNT`).
fn desk() -> Result<(), Failure> {
    use std::os::unix::fs::MetadataExt;
    let asked: Vec<String> = env::args().skip(2).collect();
    if let ["allow", account] = asked.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        let number = account_numbered(account)
            .ok_or_else(|| format!("There is no account called {account} on this machine."))?;
        let written =
            PathBuf::from(format!("{}{DESK_OF}", setting("ORBITE_ROOT").unwrap_or_default()));
        // Read by the Agent's service and by the helper of every session,
        // whatever root's own habits for the files it makes.
        let write = || {
            use std::os::unix::fs::PermissionsExt;
            if let Some(folder) = written.parent() {
                fs::create_dir_all(folder)?;
                fs::set_permissions(folder, fs::Permissions::from_mode(0o755))?;
            }
            fs::write(&written, format!("{number}\n"))?;
            fs::set_permissions(&written, fs::Permissions::from_mode(0o644))
        };
        write().map_err(|error| {
            format!("{} could not be written: {error}. Run this as root.", written.display())
        })?;
        println!(
            "The desktop of {account} may be offered to the agent, once they run: orbite desk\n\
             The agent takes this in when it next starts."
        );
        return Ok(());
    }
    if !asked.is_empty() {
        return Err("Usage: orbite desk | sudo orbite desk allow ACCOUNT".into());
    }
    // The folder of the Orbite installed here, which its Owner cannot read
    // but can see is there, and whose account the Agent's service runs as.
    let installed = match setting("ORBITE_HOME") {
        Some(home) => PathBuf::from(home),
        None => {
            PathBuf::from(format!("{}/var/lib/orbite", setting("ORBITE_ROOT").unwrap_or_default()))
        }
    };
    let agent_of = |folder: &PathBuf| fs::metadata(folder).map(|folder| folder.uid());
    let agent = agent_of(&installed).map_err(|_| "Orbite is not installed on this machine.")?;
    let account = required("USER").or_else(|_| required("LOGNAME"))?;
    // Only the account written down offers its desktop: the session of
    // anyone else on this machine lends nothing, to whoever would ask.
    let me = fs::metadata("/proc/self").map(|myself| myself.uid()).ok();
    if me.is_none() || desk_account() != me {
        return Err(not_the_account(&account).into());
    }
    let display = setting_of_the_session("DISPLAY");
    let wayland = setting_of_the_session("WAYLAND_DISPLAY").is_some()
        || setting_of_the_session("XDG_SESSION_TYPE").as_deref() == Some("wayland");
    let session = match (&display, wayland) {
        (_, true) => "wayland",
        (Some(_), false) => "x11",
        (None, false) => {
            return Err(
                "This is not a graphical session: there is no desktop here to offer.".into()
            );
        }
    };
    let screen = orbite::Screen::at(display.unwrap_or_default());
    // The Agent's service may be starting, or starting again: it is waited
    // for, as long as this session lasts and Orbite is installed.
    loop {
        if agent_of(&installed).ok() != Some(agent) {
            // Orbite was removed, or is another's now: nothing to lend to.
            return Ok(());
        }
        match orbite::offer_this_computer(orbite::DESK_SOCKET, agent, &account, session, &screen) {
            Ok(()) | Err(orbite::NotOffered::NobodyListening) => {
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
            // Something else holds the place the service listens at: it is
            // told nothing, and the service is waited for.
            Err(orbite::NotOffered::NotOrbite | orbite::NotOffered::Other(_)) => {
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
            Err(orbite::NotOffered::NotTheOwner) => {
                return Err(not_the_account(&account).into());
            }
            Err(orbite::NotOffered::Wayland) => {
                return Err(
                    "This session is a Wayland one, whose desktop Orbite cannot be lent yet: \
                            the agent keeps a computer of its own. Sessions on Xorg can be lent."
                        .into(),
                );
            }
        }
    }
}

/// What `account` is told when theirs is not the desktop Orbite takes.
fn not_the_account(account: &str) -> String {
    format!(
        "Orbite takes the desktop of the account it was installed by, which is not {account}. \
         To have it take yours: sudo orbite desk allow {account}"
    )
}

/// A setting of the graphical session this was started in.
fn setting_of_the_session(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// Where it is written which computer the Agent uses: chosen once, when
/// Orbite is installed, and out of the Agent's reach.
const COMPUTER_OF: &str = "/usr/local/share/orbite/computer";

/// The computer chosen for the Agent: `this` machine's desktop, or one of
/// its `own`. None where nobody chose.
fn computer_chosen() -> Option<String> {
    let written = format!("{}{COMPUTER_OF}", setting("ORBITE_ROOT").unwrap_or_default());
    let chosen = fs::read_to_string(written).ok()?.trim().to_owned();
    matches!(chosen.as_str(), "this" | "own").then_some(chosen)
}

/// Asks, once, which computer the Agent will use, and writes it down
/// where only root writes. The installer asks this; it is not asked again.
fn choose_computer() -> Result<(), Failure> {
    use orbite::{Choice, Front, Picked};
    if let Some(chosen) = computer_chosen() {
        println!(
            "Your agent uses {}: that was chosen when Orbite was installed, and stays. \
             Install Orbite again to choose again.",
            if chosen == "own" { "a computer of its own" } else { "this machine's own desktop" }
        );
        return Ok(());
    }
    let choices = [
        Choice::of("This machine's own desktop")
            .hinted("it works where you see it, with your programs")
            .also(&["this"]),
        Choice::of("A computer of its own")
            .hinted("apart: nothing of yours is there")
            .also(&["own"]),
    ];
    let question = "Which computer will your agent use? This is chosen once.";
    let asked = |front: &mut dyn Front| front.choose(question, &choices, 0, None);
    let picked = if orbite::Welcome::fits_here()
        && let Ok(mut screen) = orbite::Welcome::open()
    {
        asked(&mut screen)
    } else {
        let typed = std::io::BufReader::new(std::io::stdin());
        asked(&mut orbite::Lines::new(typed, std::io::stdout()))
    };
    let chosen = match picked {
        Ok(Picked::Choice(1)) => "own",
        Ok(_) => "this",
        Err(_) => return Err("Nothing was chosen.".into()),
    };
    let written =
        PathBuf::from(format!("{}{COMPUTER_OF}", setting("ORBITE_ROOT").unwrap_or_default()));
    let write = || {
        use std::os::unix::fs::PermissionsExt;
        if let Some(folder) = written.parent() {
            fs::create_dir_all(folder)?;
            fs::set_permissions(folder, fs::Permissions::from_mode(0o755))?;
        }
        fs::write(&written, format!("{chosen}\n"))?;
        fs::set_permissions(&written, fs::Permissions::from_mode(0o644))
    };
    write().map_err(|error| {
        format!("{} could not be written: {error}. Run this as root.", written.display()).into()
    })
}

/// Where the installer, or root after it, wrote whose desktop the Agent may
/// be lent: a place the Agent's own account cannot write to.
const DESK_OF: &str = "/usr/local/share/orbite/desk";

/// The number of the account whose desktop the Agent may be lent, when one
/// was written down.
fn desk_account() -> Option<u32> {
    let written = format!("{}{DESK_OF}", setting("ORBITE_ROOT").unwrap_or_default());
    fs::read_to_string(written).ok()?.trim().parse().ok()
}

/// The number of the account called `name` on this machine.
fn account_numbered(name: &str) -> Option<u32> {
    let accounts = fs::read_to_string("/etc/passwd").unwrap_or_default();
    let written = accounts.lines().find_map(|line| {
        let mut parts = line.split(':');
        (parts.next()? == name).then(|| parts.nth(1)?.parse().ok()).flatten()
    });
    // An account kept elsewhere than in this machine's own file (a
    // directory of the network) is known to the system all the same.
    written.or_else(|| {
        if name.starts_with('-') {
            return None;
        }
        let asked =
            std::process::Command::new(orbite::system_program("id")).args(["-u", name]).output();
        String::from_utf8_lossy(&asked.ok().filter(|said| said.status.success())?.stdout)
            .trim()
            .parse()
            .ok()
    })
}

/// Lists the devices let into the web interface, or turns one away.
fn devices() -> Result<(), Failure> {
    let home = home()?;
    let asked: Vec<String> = env::args().skip(2).collect();
    match asked.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => {
            let all = orbite::interface_devices(&home)?;
            if all.is_empty() {
                println!("No device was let in. Let one in: orbite pair");
            }
            for device in all {
                let seen = chrono::DateTime::from_timestamp(device.seen as i64, 0)
                    .map(|seen| {
                        seen.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()
                    })
                    .unwrap_or_default();
                println!("{}   (last seen {seen})", device.name);
            }
        }
        ["remove", ..] => {
            let name = asked[1..].join(" ");
            if orbite::turn_away(&home, &name)? {
                println!("{name} is turned away: it is let in no more.");
            } else {
                return Err(format!("No device is called {name}.").into());
            }
        }
        _ => return Err("Usage: orbite devices | devices remove NAME".into()),
    }
    Ok(())
}

/// Shows every connection of this installation, or adds or removes a connector.
fn connections() -> Result<(), Failure> {
    let home = home()?;
    let asked: Vec<String> = env::args().skip(2).collect();
    match asked.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => {
            let language = Language::of(Config::load(&home)?.language.as_deref());
            println!(
                "{}\n{}\n",
                language.say(
                    "AIs, in the order they are asked:",
                    "IA, dans l'ordre où elles sont sollicitées :"
                ),
                orbite::providers_shown(&home, language, &[])
            );
            let off = orbite::connectors_turned_off(&home);
            let connectors: Vec<String> = orbite::connectors(&home)
                .into_iter()
                .map(|name| {
                    if off.contains(&name) {
                        format!("{name} ({})", language.say("turned off", "désactivé"))
                    } else {
                        name
                    }
                })
                .collect();
            println!(
                "{} {}",
                language.say("Connectors:", "Connecteurs :"),
                if connectors.is_empty() {
                    language.say("none", "aucun").to_owned()
                } else {
                    connectors.join(", ")
                }
            );
            println!(
                "{}",
                language.say(
                    "\nIn the conversation, /connections shows how each stands and signs in to those that want it.\nAdd a connector: orbite connections add NAME ADDRESS | remove NAME\nAdd an AI: orbite providers add",
                    "\nDans la discussion, /connections montre l'état de chacun et connecte ceux qui le demandent.\nAjouter un connecteur : orbite connections add NOM ADRESSE | remove NOM\nAjouter une IA : orbite providers add",
                )
            );
            return Ok(());
        }
        ["add", name, address] => {
            orbite::add_connector(&home, name, address)?;
            println!(
                "{name} is added. Sign in to it, if it wants that, from the conversation: /mcp connect {name}"
            );
        }
        ["remove", name] => {
            orbite::remove_connector(&home, name)?;
            println!("{name} is removed.");
        }
        _ => return Err("Usage: orbite connections | add NAME ADDRESS | remove NAME".into()),
    }
    println!(
        "It applies at the next start of Orbite (as a service: sudo systemctl restart orbite)."
    );
    Ok(())
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
        let mut setup = Setup::in_home(home()?, typed, std::io::stdout())
            .turning_the_interface_on_with(|| switch_the_interface(true));
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
        Some("interface") => Some(SetupStep::Interface),
        Some("name") => Some(SetupStep::Name),
        Some("look") => Some(SetupStep::Look),
        Some(_) => {
            return Err(
                "Usage: orbite setup [language|provider|telegram|interface|name|look]".into()
            );
        }
    };
    // On a screen of its own where the terminal can show one; as lines
    // read and written anywhere else.
    if orbite::Welcome::fits_here()
        && let Ok(screen) = orbite::Welcome::open()
    {
        let setup = Setup::shown_with(home()?, screen)
            .where_tailscale_is(orbite::tailscale().is_some())
            .where_the_public_address_is(orbite::public_address())
            .turning_the_interface_on_with(|| switch_the_interface(true));
        return Ok(setup.run(only).await?);
    }
    let typed = std::io::BufReader::new(std::io::stdin());
    let mut setup = Setup::in_home(home()?, typed, std::io::stdout())
        .where_tailscale_is(orbite::tailscale().is_some())
        .where_the_public_address_is(orbite::public_address())
        .turning_the_interface_on_with(|| switch_the_interface(true));
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        setup = setup.reading_secrets_with(|question| rpassword::prompt_password(question));
    }
    Ok(setup.run(only).await?)
}

/// Turns the web interface on or off by running `orbite interface on|off`,
/// as root where the Agent's account was given that: it is what may have
/// Tailscale serve it to the Owner's other devices.
fn switch_the_interface(on: bool) -> Result<(), String> {
    let program = env::current_exe().map_err(|error| error.to_string())?;
    let quietly = |command: &mut std::process::Command| {
        command
            .args(["interface", if on { "on" } else { "off" }])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|ended| ended.success())
    };
    let as_root = quietly(std::process::Command::new("sudo").arg("-n").arg(&program));
    if as_root || quietly(&mut std::process::Command::new(&program)) {
        return Ok(());
    }
    Err("The web interface could not be changed.".into())
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
    let keep_computer = arguments.iter().any(|argument| argument == "--keep-computer");
    let root = setting("ORBITE_ROOT").unwrap_or_default();
    let data = PathBuf::from(format!("{root}/var/lib/orbite"));
    let brought = Brought::written_in(&root);
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
        println!("This removes Orbite from this machine. {losing}");
        if !brought.packages.is_empty() && !keep_computer {
            println!(
                "What the installer brought for the agent's computer is removed too ({}); add \
                 --keep-computer to keep it.",
                brought.packages.join(", ")
            );
        }
        println!("Type yes to go on:");
        let mut typed = String::new();
        std::io::stdin().read_line(&mut typed)?;
        if typed.trim() != "yes" {
            return Err("Nothing was removed.".into());
        }
    }
    let systemctl = |arguments: &[&str]| {
        let _ = std::process::Command::new(orbite::system_program("systemctl"))
            .args(arguments)
            .status();
    };
    systemctl(&["disable", "--now", "orbite"]);
    systemctl(&["disable", "--now", "orbite-update.timer"]);
    // What Tailscale serves for the web interface goes with it.
    if let Ok(Config { interface_reached_at: Some(_), .. }) = Config::load(&data)
        && let (Some(tailscale), Ok(local)) = (orbite::tailscale(), DEFAULT_INTERFACE.parse())
    {
        let _ = orbite::close_on_tailscale(&tailscale, local);
    }
    // The data first: were it to fail, the program is still there to try again.
    if !keep && data.exists() {
        fs::remove_dir_all(&data)?;
    }
    for file in [
        "/etc/systemd/system/orbite.service",
        "/etc/systemd/system/orbite.service.replaced",
        "/var/lib/systemd/timers/stamp-orbite-update.timer",
        "/etc/systemd/system/orbite-update.service",
        "/etc/systemd/system/orbite-update.timer",
        "/etc/sudoers.d/orbite",
        "/etc/xdg/autostart/orbite-desk.desktop",
        DESK_OF,
        COMPUTER_OF,
        "/usr/local/bin/orbite.previous",
        "/usr/local/bin/orbite.refused",
        "/usr/local/bin/orbite.updating",
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
    // The folder of what the installer wrote down, when nothing is left in it.
    if let Some(folder) = PathBuf::from(format!("{root}{DESK_OF}")).parent() {
        let _ = fs::remove_dir(folder);
    }
    systemctl(&["daemon-reload"]);
    if keep {
        // The data stays, given to root before the account goes: left to
        // the account's number, a later account given that number would
        // read the Agent's secrets.
        let _ = std::process::Command::new(orbite::system_program("chown"))
            .args(["-R", "root:root"])
            .arg(&data)
            .status();
    }
    let _ = std::process::Command::new(orbite::system_program("userdel")).arg("orbite").status();
    // Some systems give every account a mailbox, which the account leaves behind.
    for mailbox in ["/var/spool/mail/orbite", "/var/mail/orbite"] {
        let _ = fs::remove_file(format!("{root}{mailbox}"));
    }
    if keep {
        println!("Orbite is removed. The Agent's data is kept in {}.", data.display());
    } else {
        println!("Orbite is removed, with the Agent's data.");
    }
    brought.taken_away(&root, keep_computer);
    Ok(())
}

/// What the installer brought for the Agent's Computer that the machine did
/// not have before, as the installer wrote it down.
#[derive(Default)]
struct Brought {
    manager: String,
    packages: Vec<String>,
    files: Vec<String>,
    /// Everyday tools the installer had to install: never removed.
    tools: Vec<String>,
    written: bool,
}

impl Brought {
    const WRITTEN: &str = "/usr/local/share/orbite/brought";

    fn written_in(root: &str) -> Self {
        let Ok(text) = fs::read_to_string(format!("{root}{}", Self::WRITTEN)) else {
            return Self::default();
        };
        let mut brought = Self { written: true, ..Self::default() };
        for line in text.lines() {
            match line.split_once(' ') {
                Some(("manager", name)) => brought.manager = name.to_owned(),
                // A name is a package's or a file's own, never an option.
                Some(("package", name)) if !name.starts_with('-') && !name.contains(' ') => {
                    brought.packages.push(name.to_owned())
                }
                Some(("file", path)) if path.starts_with('/') && !path.contains("..") => {
                    brought.files.push(path.to_owned())
                }
                Some(("tool", name)) => brought.tools.push(name.to_owned()),
                _ => {}
            }
        }
        brought
    }

    /// Removes what was brought, or says what is left when it is to be kept
    /// or cannot be removed. The machine is then as it was before Orbite.
    fn taken_away(&self, root: &str, keep: bool) {
        if !self.written {
            println!(
                "The packages installed for the agent's computer (Firefox, Xvfb, xdotool, fonts) were \
                 left: remove them with your system's own tools if nothing else uses them."
            );
            return;
        }
        let written = PathBuf::from(format!("{root}{}", Self::WRITTEN));
        let _ = fs::remove_file(&written);
        if let Some(folder) = written.parent() {
            let _ = fs::remove_dir(folder);
        }
        if !self.tools.is_empty() {
            println!(
                "Left on the machine, as other things may use them by now: {} (installed by Orbite's \
                 installer).",
                self.tools.join(", ")
            );
        }
        if self.packages.is_empty() && self.files.is_empty() {
            return;
        }
        if keep {
            println!(
                "Kept, as asked, what was brought for the agent's computer: {}.",
                self.packages.join(" ")
            );
            return;
        }
        // A package goes only when nothing but what was brought goes with
        // it: what the Owner installed since and that needs one is never
        // taken along. Several rounds, as one may be freed by another going.
        let mut left = self.packages.clone();
        let mut gone: Vec<String> = Vec::new();
        for _ in 0..4 {
            let free: Vec<String> =
                left.iter().filter(|package| self.goes_alone(package)).cloned().collect();
            let removed: Vec<String> = if self.manager == "pacman" {
                // pacman refuses by itself what something else needs.
                free.into_iter()
                    .filter(|package| self.remove(std::slice::from_ref(package)))
                    .collect()
            } else if !free.is_empty() && self.remove(&free) {
                free
            } else {
                Vec::new()
            };
            if removed.is_empty() {
                break;
            }
            left.retain(|package| !removed.contains(package));
            gone.extend(removed);
        }
        for file in &self.files {
            let _ = fs::remove_file(format!("{root}{file}"));
        }
        if !gone.is_empty() {
            println!(
                "What was brought for the agent's computer is removed too: {}.",
                gone.join(" ")
            );
        }
        if !left.is_empty() {
            println!(
                "Left on the machine, as something else there needs it or it could not be removed: {}.",
                left.join(" ")
            );
        }
    }

    /// True when removing `package` would take nothing with it but what
    /// was brought.
    fn goes_alone(&self, package: &str) -> bool {
        let quietly = |program: &str, arguments: &[&str]| {
            std::process::Command::new(program)
                .args(arguments)
                .env("LC_ALL", "C")
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
        };
        match self.manager.as_str() {
            "apt-get" => quietly("apt-get", &["-s", "purge", package]).is_ok_and(|tried| {
                tried.status.success()
                    && String::from_utf8_lossy(&tried.stdout)
                        .lines()
                        .filter_map(|line| line.strip_prefix("Purg "))
                        .filter_map(|line| line.split_whitespace().next())
                        .all(|going| self.packages.iter().any(|brought| brought == going))
            }),
            "dnf" | "zypper" => {
                // What needs it and came in the same installation as it came
                // with it, and goes with it; what was installed since is
                // the Owner's, and keeps it.
                let came = |package: &str| {
                    quietly("rpm", &["-q", "--qf", "%{INSTALLTID}\\n", package])
                        .ok()
                        .filter(|asked| asked.status.success())
                        .map(|asked| String::from_utf8_lossy(&asked.stdout).trim().to_owned())
                };
                let Ok(tried) = std::process::Command::new("rpm")
                    .args(["-e", "--test", package])
                    .env("LC_ALL", "C")
                    .stdin(std::process::Stdio::null())
                    .output()
                else {
                    return false;
                };
                if tried.status.success() {
                    return true;
                }
                let said = String::from_utf8_lossy(&tried.stderr).into_owned();
                let needing: Vec<&str> = said
                    .lines()
                    .filter(|line| line.contains("is needed by"))
                    .filter_map(|line| line.split_whitespace().last())
                    .collect();
                let with = came(package);
                !needing.is_empty()
                    && with.is_some()
                    && needing.iter().all(|needs| came(needs) == with)
            }
            "pacman" => true,
            _ => false,
        }
    }

    /// Removes `packages` and what came with them that nothing needs any
    /// more: only what their removal leaves unneeded, never what the machine
    /// already had lying unneeded before.
    fn remove(&self, packages: &[String]) -> bool {
        let run = |program: &str, arguments: &[&str], more: &[String]| {
            std::process::Command::new(program)
                .args(arguments)
                .args(more)
                .env("DEBIAN_FRONTEND", "noninteractive")
                .env("LC_ALL", "C")
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
        };
        let done =
            |ran: std::io::Result<std::process::Output>| ran.is_ok_and(|ran| ran.status.success());
        match self.manager.as_str() {
            "apt-get" => {
                // What apt would clear away by itself, before and after.
                // `None` when apt could not say: nothing more is then removed.
                let unneeded = || -> Option<Vec<String>> {
                    let tried = run("apt-get", &["-s", "autoremove"], &[]).ok()?;
                    tried.status.success().then(|| {
                        String::from_utf8_lossy(&tried.stdout)
                            .lines()
                            .filter_map(|line| line.strip_prefix("Remv "))
                            .filter_map(|line| line.split_whitespace().next())
                            .map(str::to_owned)
                            .collect()
                    })
                };
                let before = unneeded();
                let waits = ["-o", "DPkg::Lock::Timeout=600", "purge", "-y", "-qq"];
                if !done(run("apt-get", &waits, packages)) {
                    return false;
                }
                if let (Some(before), Some(after)) = (before, unneeded()) {
                    let left: Vec<String> =
                        after.into_iter().filter(|package| !before.contains(package)).collect();
                    if !left.is_empty() {
                        let _ = run("apt-get", &waits, &left);
                    }
                }
                true
            }
            "dnf" => done(run("dnf", &["remove", "-y", "-q"], packages)),
            "pacman" => done(run("pacman", &["-Rns", "--noconfirm"], packages)),
            "zypper" => {
                done(run("zypper", &["--non-interactive", "remove", "--clean-deps"], packages))
            }
            _ => false,
        }
    }
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
    let updater = Updater::new(env!("CARGO_PKG_VERSION"), release, key)
        .installed_at(program.clone())
        .telling_the_owner_in(home.clone());
    // Run by hand, in a terminal: each thing is shown as it is done. Run by
    // the daily timer, one line says what happened.
    if !orbite::Welcome::fits_here() {
        println!("{}", updater.run(&Systemd).await?);
        return Ok(());
    }
    let french = Config::load(&home).is_ok_and(|config| config.language.as_deref() == Some("fr"));
    let say = move |english: &str, french_words: &str| {
        if french { french_words.to_owned() } else { english.to_owned() }
    };
    let steps = std::sync::Arc::new(Steps::default());
    println!("\n  \x1b[1mo r b i t e\x1b[0m\n");
    let watching = steps.clone();
    let done = updater
        .watched_by(move |what| {
            let (step, version) = what.split_once(' ').unwrap_or((what, ""));
            watching.begin(&match step {
                "looking" => say("Looking for a new version", "Recherche d'une nouvelle version"),
                "downloading" => {
                    format!("{} {version}", say("Downloading Orbite", "Téléchargement d'Orbite"))
                }
                "installing" => say(
                    "Signed by Orbite's key: installing it",
                    "Bien signé par la clé d'Orbite : installation",
                ),
                _ => format!("{} {version}", say("Starting Orbite", "Démarrage d'Orbite")),
            });
        })
        .run(&Systemd)
        .await;
    let say = move |english: &str, french_words: &str| {
        if french { french_words.to_owned() } else { english.to_owned() }
    };
    let done = match done {
        Ok(done) => done,
        Err(error) => {
            steps.end(
                false,
                &say("The update did not go through.", "La mise à jour n'a pas abouti."),
            );
            return Err(error.into());
        }
    };
    let running = env!("CARGO_PKG_VERSION");
    if let Some(version) =
        done.strip_prefix("Updated to ").map(|version| version.trim_end_matches('.'))
    {
        steps.end(
            true,
            &format!(
                "{} {version} ({} {running})",
                say("Orbite is now on", "Orbite est maintenant en"),
                say("it was on", "c'était la"),
            ),
        );
        // What the new one brings, as it says itself.
        let brought = std::process::Command::new(&program).arg("news").output();
        let lines: Vec<String> = brought
            .ok()
            .filter(|told| told.status.success())
            .map(|told| String::from_utf8_lossy(&told.stdout).lines().map(str::to_owned).collect())
            .unwrap_or_default();
        if !lines.is_empty() {
            println!("\n  \x1b[1m{}\x1b[0m", say("What is new", "Ce qui change"));
            for line in lines.iter().take(8) {
                println!("  \x1b[2m•\x1b[0m {line}");
            }
        }
    } else if done.contains("up to date") {
        steps.end(true, &format!("Orbite {running} {}", say("is up to date.", "est à jour.")));
    } else {
        steps.end(true, &done);
    }
    println!();
    Ok(())
}

/// What is being done, shown with a mark that turns until it is.
#[derive(Default)]
struct Steps {
    turning: std::sync::Mutex<Option<Turning>>,
}

/// A mark that turns: what stops it, what draws it, and what it says.
type Turning = (std::sync::Arc<std::sync::atomic::AtomicBool>, std::thread::JoinHandle<()>, String);

impl Steps {
    /// Ticks off what was being done, and shows what is begun.
    fn begin(&self, what: &str) {
        self.settle(true);
        let over = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stop, text) = (over.clone(), what.to_owned());
        let turning = std::thread::spawn(move || {
            use std::io::Write;
            const MARKS: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut at = 0;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                print!("\r  \x1b[38;5;75m{}\x1b[0m \x1b[2m{text}\x1b[0m", MARKS[at % 10]);
                let _ = std::io::stdout().flush();
                at += 1;
                std::thread::sleep(std::time::Duration::from_millis(90));
            }
        });
        *self.turning.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some((over, turning, what.to_owned()));
    }

    /// Stops the mark; what was being done is ticked off when it went well.
    fn settle(&self, well: bool) {
        let was = self.turning.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some((over, turning, what)) = was {
            over.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = turning.join();
            let mark = if well { "\x1b[38;5;78m✓\x1b[0m" } else { "\x1b[38;5;210m✗\x1b[0m" };
            println!("\r\x1b[K  {mark} \x1b[2m{what}\x1b[0m");
        }
    }

    /// The last word: how it ended.
    fn end(&self, well: bool, said: &str) {
        self.settle(well);
        let mark = if well { "\x1b[38;5;78m✓\x1b[0m" } else { "\x1b[38;5;210m✗\x1b[0m" };
        println!("  {mark} \x1b[1m{said}\x1b[0m");
    }
}

/// What this version of Orbite brings, as its list of changes says: the
/// first words of each thing, without what marks them.
fn news() -> Vec<String> {
    const CHANGES: &str = include_str!("../CHANGELOG.md");
    let heading = format!("## {}", env!("CARGO_PKG_VERSION"));
    let Some(from) = CHANGES.find(&heading) else { return Vec::new() };
    let entry = &CHANGES[from + heading.len()..];
    let entry = entry.find("\n## ").map_or(entry, |end| &entry[..end]);
    entry
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .map(|line| {
            // The part in bold says it; failing that, the first sentence.
            let plain = line.replace("**", "");
            let told = match line.strip_prefix("**").and_then(|rest| rest.split_once("**")) {
                Some((bold, _)) => bold.trim_end_matches(['.', ':']).to_owned(),
                None => plain.split(". ").next().unwrap_or(&plain).to_owned(),
            };
            told.chars().take(110).collect()
        })
        .collect()
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
    match fs::read_dir(&installed) {
        Ok(_) => return Ok(installed),
        // Installed, and its folder is its own account's: said, rather
        // than taking this for a machine where nothing was set up.
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            let asked: Vec<String> = env::args().skip(1).collect();
            return Err(format!(
                "Orbite is installed on this machine, and what it keeps is its own account's to \
                 read. Run this with sudo: sudo orbite {}",
                asked.join(" ")
            )
            .into());
        }
        Err(_) => {}
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
    // Run as root by sudo itself for an account that is not root: what that
    // account put around the command has no say in what root then does.
    #[cfg(unix)]
    {
        let (asked_by, ran) = (env::var("SUDO_UID").ok(), env::var("SUDO_COMMAND").ok());
        let for_another = env::current_exe().is_ok_and(|program| {
            orbite::run_by_sudo_for_another(asked_by.as_deref(), ran.as_deref(), &program)
        });
        // SAFETY: a plain system call.
        if for_another && unsafe { libc::geteuid() } == 0 {
            return None;
        }
    }
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}
