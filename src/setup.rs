//! The guided setup: a few questions in the terminal, each answer checked
//! for real before the next, and everything kept as it goes so that a setup
//! cut short resumes where it stopped.

use std::fmt;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{
    Config, KnownProvider, Language, Message, Provider, ProviderChoice, ProviderError, Request,
    Secrets, Speaks, connect, known_providers,
};

/// Under these names the installation's own keys are sealed. The Agent
/// never uses or lists them.
pub const PROVIDER_KEY: &str = "ORBITE_PROVIDER_KEY";
pub const TELEGRAM_TOKEN: &str = "ORBITE_TELEGRAM_TOKEN";

/// Names offered for the Agent.
const NAMES: &[&str] = &["Nova", "Sol", "Mira", "Echo", "Orbi"];

/// A part of setup that can be done again on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Language,
    Provider,
    Telegram,
    Name,
}

/// Why setup stopped before its end.
#[derive(Debug)]
pub struct SetupError(String);

impl fmt::Display for SetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SetupError {}

fn stopped(error: impl fmt::Display) -> SetupError {
    SetupError(error.to_string())
}

/// Reads an answer without showing it as it is typed.
type ReadHidden = Box<dyn FnMut(&str) -> std::io::Result<String> + Send>;

pub struct Setup<R, W> {
    home: PathBuf,
    typed: R,
    shown: W,
    /// Reads what must not be shown as it is typed; by default, a line of `typed`.
    hidden: Option<ReadHidden>,
    providers_at: Option<String>,
    telegram_at: String,
    code: Option<String>,
    look_every: Duration,
    give_up_after: Duration,
    http: reqwest::Client,
}

impl<R: BufRead, W: Write> Setup<R, W> {
    /// Setup of the installation whose folder is `home`, asking in `shown`
    /// and reading answers from `typed`.
    pub fn in_home(home: impl Into<PathBuf>, typed: R, shown: W) -> Self {
        Self {
            home: home.into(),
            typed,
            shown,
            hidden: None,
            providers_at: None,
            telegram_at: "https://api.telegram.org".into(),
            code: None,
            look_every: Duration::from_secs(2),
            give_up_after: Duration::from_secs(600),
            http: reqwest::Client::new(),
        }
    }

    /// Reads keys and tokens with `read`, which shows nothing of what is typed.
    pub fn reading_secrets_with(
        mut self,
        read: impl FnMut(&str) -> std::io::Result<String> + Send + 'static,
    ) -> Self {
        self.hidden = Some(Box::new(read));
        self
    }

    /// Reaches every Provider at this address rather than its own.
    pub fn reaching_providers_at(mut self, address: impl Into<String>) -> Self {
        self.providers_at = Some(address.into());
        self
    }

    /// Reaches Telegram at this address rather than its own.
    pub fn reaching_telegram_at(mut self, address: impl Into<String>) -> Self {
        self.telegram_at = address.into().trim_end_matches('/').to_owned();
        self
    }

    /// Uses this code to link the Owner's account, rather than one drawn at random.
    pub fn linking_with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// How often Telegram is looked at for the Owner's code.
    pub fn waiting_between_looks(mut self, pace: Duration) -> Self {
        self.look_every = pace;
        self
    }

    /// How long the Owner's code is waited for.
    pub fn giving_up_linking_after(mut self, patience: Duration) -> Self {
        self.give_up_after = patience;
        self
    }

    /// Goes through what is not set up yet, or through `only` again.
    pub async fn run(mut self, only: Option<Step>) -> Result<(), SetupError> {
        let mut config = Config::load(&self.home).map_err(stopped)?;
        let secrets = Secrets::kept_in(&self.home);
        let has = |name: &str| secrets.get(name).map(|kept| kept.is_some()).map_err(stopped);
        let wanted = |step: Step, done: bool| only.map_or(!done, |only| only == step);

        if wanted(Step::Language, config.language.is_some()) {
            self.language(&mut config)?;
        }
        let language = Language::of(config.language.as_deref());
        if wanted(Step::Provider, config.provider.is_some()) {
            self.provider(&mut config, &secrets, language).await?;
        }
        if wanted(Step::Telegram, config.telegram_owner.is_some() && has(TELEGRAM_TOKEN)?) {
            self.telegram(&mut config, &secrets, language).await?;
        }
        if wanted(Step::Name, config.agent_name.is_some()) {
            self.name(&mut config, language)?;
        }
        if only.is_none() {
            self.greet(&config, &secrets, language).await?;
        }
        Ok(())
    }

    fn language(&mut self, config: &mut Config) -> Result<(), SetupError> {
        let chosen = loop {
            let answer = self.ask("Language / Langue\n  1. English\n  2. Français\n> ")?;
            match answer.to_lowercase().as_str() {
                "1" | "en" | "english" => break "en",
                "2" | "fr" | "français" | "francais" => break "fr",
                _ => {}
            }
        };
        config.language = Some(chosen.into());
        config.save(&self.home).map_err(stopped)
    }

    async fn provider(
        &mut self,
        config: &mut Config,
        secrets: &Secrets,
        language: Language,
    ) -> Result<(), SetupError> {
        self.say(language.say(
            "\nWhich AI should your agent think with?",
            "\nAvec quelle IA votre agent doit-il réfléchir ?",
        ))?;
        for (place, known) in known_providers().iter().enumerate() {
            self.say(&format!("  {}. {} ({})", place + 1, known.label, known.name))?;
        }
        self.say(language.say(
            "  or \"custom\" for any server speaking OpenAI's protocol",
            "  ou « custom » pour tout serveur parlant le protocole d'OpenAI",
        ))?;
        let known = loop {
            let answer =
                self.ask(language.say("Number or name > ", "Numéro ou nom > "))?.to_lowercase();
            let by_number = answer
                .parse::<usize>()
                .ok()
                .and_then(|number| known_providers().get(number.wrapping_sub(1)));
            let by_name = known_providers().iter().find(|known| known.name == answer);
            if let Some(known) = by_number.or(by_name) {
                break *known;
            }
            if answer == "custom" {
                break KnownProvider {
                    name: "custom",
                    label: "custom",
                    speaks: Speaks::OpenAi,
                    api_root: "",
                    needs_key: true,
                };
            }
        };
        let address = match (&self.providers_at, known.name) {
            (Some(address), _) => Some(address.clone()),
            (None, "custom") => Some(self.ask(
                language.say("Its address (https://…/v1) > ", "Son adresse (https://…/v1) > "),
            )?),
            (None, _) => None,
        };
        let root = address.clone().unwrap_or_else(|| known.api_root.to_owned());

        // The key first, checked for real, then the model.
        let key = loop {
            if !known.needs_key {
                break None;
            }
            let key = self.ask_hidden(
                language.say("API key (not shown) > ", "Clé d'API (non affichée) > "),
            )?;
            match self.check(&known, &key, "", &root).await {
                Err(ProviderError::Rejected(reason)) => self.say(&format!(
                    "{} {reason}",
                    language.say(
                        "The provider refused this key:",
                        "Le fournisseur a refusé cette clé :"
                    )
                ))?,
                Err(ProviderError::Unreachable(reason)) => self.say(&format!(
                    "{} {reason}",
                    language.say(
                        "The provider could not be reached:",
                        "Le fournisseur est injoignable :"
                    )
                ))?,
                // Anything else is about the model, which is not chosen yet.
                _ => break Some(key),
            }
        };
        let offered = self.models(&known, key.as_deref(), &root).await;
        if !offered.is_empty() {
            self.say(language.say("\nModels:", "\nModèles :"))?;
            for (place, model) in offered.iter().enumerate() {
                self.say(&format!("  {}. {model}", place + 1))?;
            }
        }
        let model = loop {
            let answer =
                self.ask(language.say("Model (number or name) > ", "Modèle (numéro ou nom) > "))?;
            let model = answer
                .parse::<usize>()
                .ok()
                .and_then(|number| offered.get(number.wrapping_sub(1)).cloned())
                .unwrap_or(answer);
            match self.check(&known, key.as_deref().unwrap_or_default(), &model, &root).await {
                Ok(()) => break model,
                Err(error) => self.say(&format!(
                    "{} {}",
                    language.say("That did not work:", "Cela n'a pas fonctionné :"),
                    crate::agent::explain(&error, language)
                ))?,
            }
        };

        if let Some(key) = &key {
            secrets.set(PROVIDER_KEY, key).map_err(stopped)?;
        }
        config.provider = Some(ProviderChoice { name: known.name.into(), model, address });
        config.save(&self.home).map_err(stopped)?;
        self.say(language.say("The AI answers. ✓", "L'IA répond. ✓"))
    }

    /// Asks the Provider for one word, to see that the key and model work.
    async fn check(
        &self,
        known: &KnownProvider,
        key: &str,
        model: &str,
        root: &str,
    ) -> Result<(), ProviderError> {
        let model = if model.is_empty() { "check" } else { model };
        let provider =
            connect(known.name, Some(key), model, Some(root)).map_err(ProviderError::Failed)?;
        let conversation = [Message::from_owner("Reply with the single word: ok")];
        let request = Request {
            instructions: "",
            conversation: &conversation,
            actions: &[],
            images: &[],
            light: true,
        };
        provider.reply(request).await.map(|_| ())
    }

    /// The models the Provider offers, when it says; a few of them.
    async fn models(&self, known: &KnownProvider, key: Option<&str>, root: &str) -> Vec<String> {
        if known.speaks == Speaks::Anthropic {
            return ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"]
                .map(String::from)
                .to_vec();
        }
        let asking = self
            .http
            .get(format!("{}/models", root.trim_end_matches('/')))
            .timeout(Duration::from_secs(20));
        let asking = match key {
            Some(key) => asking.bearer_auth(key),
            None => asking,
        };
        let Ok(response) = asking.send().await else { return Vec::new() };
        let Ok(listed) = response.json::<Value>().await else { return Vec::new() };
        let models = listed["data"].as_array().cloned().unwrap_or_default();
        models.iter().filter_map(|model| model["id"].as_str().map(str::to_owned)).take(15).collect()
    }

    async fn telegram(
        &mut self,
        config: &mut Config,
        secrets: &Secrets,
        language: Language,
    ) -> Result<(), SetupError> {
        self.say(language.say(
            "\nYou will talk to your agent in Telegram, through a bot of your own:\n  \
             1. In Telegram, open a chat with @BotFather.\n  \
             2. Send /newbot and follow its two questions (a name, then a username ending in \"bot\").\n  \
             3. BotFather gives you a token: paste it here.",
            "\nVous parlerez à votre agent dans Telegram, par un bot qui vous appartient :\n  \
             1. Dans Telegram, ouvrez une discussion avec @BotFather.\n  \
             2. Envoyez /newbot et répondez à ses deux questions (un nom, puis un identifiant finissant par « bot »).\n  \
             3. BotFather vous donne un jeton : collez-le ici.",
        ))?;
        let (token, username) = loop {
            let token = self.ask_hidden(
                language.say("Bot token (not shown) > ", "Jeton du bot (non affiché) > "),
            )?;
            match self.bot(&token, "getMe", json!({})).await {
                Ok(bot) => {
                    break (token, bot["username"].as_str().unwrap_or("your bot").to_owned());
                }
                Err(reason) => self.say(&format!(
                    "{} ({reason})",
                    language.say(
                        "Telegram does not know this token. Copy it again from @BotFather",
                        "Telegram ne connaît pas ce jeton. Recopiez-le depuis @BotFather",
                    )
                ))?,
            }
        };
        secrets.set(TELEGRAM_TOKEN, &token).map_err(stopped)?;

        // Whoever sends the code is the Owner: no identifier to look up or type.
        let code = self.code.clone().unwrap_or_else(|| {
            let mut bytes = [0u8; 4];
            let _ = getrandom::fill(&mut bytes);
            format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000)
        });
        self.say(&format!(
            "{} @{username} ✓\n{} {code}",
            language.say("Bot recognised:", "Bot reconnu :"),
            language.say(
                "Now open its chat in Telegram and send it this code:",
                "Ouvrez maintenant sa discussion dans Telegram et envoyez-lui ce code :",
            )
        ))?;
        let started = Instant::now();
        let mut next_update = 0;
        let owner = 'linking: loop {
            if started.elapsed() > self.give_up_after {
                return Err(SetupError(
                    language
                        .say(
                            "No code arrived. Run setup again when you are ready to send it.",
                            "Aucun code n'est arrivé. Relancez la configuration quand vous serez prêt à l'envoyer.",
                        )
                        .into(),
                ));
            }
            let asking =
                json!({ "offset": next_update, "timeout": 0, "allowed_updates": ["message"] });
            if let Ok(updates) = self.bot(&token, "getUpdates", asking).await {
                for update in updates.as_array().cloned().unwrap_or_default() {
                    next_update = update["update_id"].as_i64().unwrap_or(0) + 1;
                    let message = &update["message"];
                    let sender = message["from"]["id"].as_i64();
                    if message["text"].as_str().map(str::trim) == Some(code.as_str())
                        && message["chat"]["type"] == "private"
                        && let Some(sender) = sender
                    {
                        break 'linking sender;
                    }
                }
            }
            tokio::time::sleep(self.look_every).await;
        };
        // Telegram keeps what it delivered until the next is asked for: ask,
        // so that the code is not waiting for the Agent as a message.
        let seen = json!({ "offset": next_update, "timeout": 0, "allowed_updates": ["message"] });
        let _ = self.bot(&token, "getUpdates", seen).await;
        config.telegram_owner = Some(owner);
        config.save(&self.home).map_err(stopped)?;
        let linked = language.say(
            "Linked: from now on I listen to you here, and to nobody else.",
            "Lié : désormais je vous écoute ici, et personne d'autre.",
        );
        let _ = self.bot(&token, "sendMessage", json!({ "chat_id": owner, "text": linked })).await;
        self.say(
            language.say("Your Telegram account is linked. ✓", "Votre compte Telegram est lié. ✓"),
        )
    }

    fn name(&mut self, config: &mut Config, language: Language) -> Result<(), SetupError> {
        self.say(language.say(
            "\nWhat will you call your agent? Some ideas:",
            "\nComment appellerez-vous votre agent ? Quelques idées :",
        ))?;
        for (place, name) in NAMES.iter().enumerate() {
            self.say(&format!("  {}. {name}", place + 1))?;
        }
        let name = loop {
            let answer = self.ask(
                language.say("Number, or a name of your own > ", "Numéro, ou un nom à vous > "),
            )?;
            let picked =
                answer.parse::<usize>().ok().and_then(|number| NAMES.get(number.wrapping_sub(1)));
            match picked {
                Some(name) => break (*name).to_owned(),
                None if answer.chars().any(char::is_alphabetic) && answer.chars().count() <= 40 => {
                    break answer;
                }
                None => {}
            }
        };
        config.agent_name = Some(name);
        config.save(&self.home).map_err(stopped)
    }

    /// The Agent's first words to its Owner, in Telegram.
    async fn greet(
        &mut self,
        config: &Config,
        secrets: &Secrets,
        language: Language,
    ) -> Result<(), SetupError> {
        let name = config.agent_name.clone().unwrap_or_else(|| "Orbite".into());
        let (Some(owner), Some(token)) =
            (config.telegram_owner, secrets.get(TELEGRAM_TOKEN).map_err(stopped)?)
        else {
            return Ok(());
        };
        let greeting = if language == Language::French {
            format!(
                "Bonjour, je suis {name}, votre agent. Je vis sur votre machine, je me souviens de ce \
                 qui compte et je vous demande avant tout ce qui change quelque chose. Dites-moi ce \
                 dont vous avez besoin."
            )
        } else {
            format!(
                "Hello, I'm {name}, your agent. I live on your machine, remember what matters, and \
                 ask before anything that changes something. Tell me what you need."
            )
        };
        let _ =
            self.bot(&token, "sendMessage", json!({ "chat_id": owner, "text": greeting })).await;
        self.say(&format!(
            "\n{} {name} {}",
            language.say("All set.", "Tout est prêt."),
            language.say("is waiting for you in Telegram.", "vous attend dans Telegram.")
        ))
    }

    /// Calls the Bot API as the bot whose token is `token`.
    async fn bot(&self, token: &str, method: &str, body: Value) -> Result<Value, String> {
        let response = self
            .http
            .post(format!("{}/bot{token}/{method}", self.telegram_at))
            .timeout(Duration::from_secs(30))
            .json(&body)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let answer: Value = response.json().await.map_err(|error| error.to_string())?;
        if answer["ok"] == true {
            Ok(answer["result"].clone())
        } else {
            Err(answer["description"].as_str().unwrap_or("refused").to_owned())
        }
    }

    fn say(&mut self, text: &str) -> Result<(), SetupError> {
        writeln!(self.shown, "{text}").and_then(|()| self.shown.flush()).map_err(stopped)
    }

    /// Shows `question` and reads the answer; nothing typed is repeated.
    fn ask(&mut self, question: &str) -> Result<String, SetupError> {
        loop {
            write!(self.shown, "{question}").and_then(|()| self.shown.flush()).map_err(stopped)?;
            let mut line = String::new();
            if self.typed.read_line(&mut line).map_err(stopped)? == 0 {
                return Err(SetupError(
                    "Setup was cut short; run it again to go on from here.".into(),
                ));
            }
            if !line.trim().is_empty() {
                return Ok(line.trim().to_owned());
            }
        }
    }

    fn ask_hidden(&mut self, question: &str) -> Result<String, SetupError> {
        match &mut self.hidden {
            Some(read) => loop {
                let answer = read(question).map_err(stopped)?;
                if !answer.trim().is_empty() {
                    return Ok(answer.trim().to_owned());
                }
            },
            None => self.ask(question),
        }
    }
}
