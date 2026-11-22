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
    Secrets, Speaks, connect_in, known_providers,
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
    /// One more Provider, asked after those already set up.
    AnotherProvider,
    Telegram,
    /// The web interface, turned on.
    Interface,
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

/// Turns the web interface on for the installation, as whoever may: where
/// Orbite is installed that takes more than the Agent's account.
type TurnOn = Box<dyn FnMut() -> Result<(), String> + Send>;

/// Reads an answer without showing it as it is typed.
type ReadHidden = Box<dyn FnMut(&str) -> std::io::Result<String> + Send>;

pub struct Setup<R, W> {
    home: PathBuf,
    typed: R,
    shown: W,
    /// Reads what must not be shown as it is typed; by default, a line of `typed`.
    hidden: Option<ReadHidden>,
    turn_interface_on: Option<TurnOn>,
    providers_at: Option<String>,
    /// True when the Provider being set up is added to those there are.
    adding: bool,
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
            turn_interface_on: None,
            providers_at: None,
            adding: false,
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

    /// Turns the web interface on with `turn_on` rather than by writing
    /// down where it is served: it then writes that down itself.
    pub fn turning_the_interface_on_with(
        mut self,
        turn_on: impl FnMut() -> Result<(), String> + Send + 'static,
    ) -> Self {
        self.turn_interface_on = Some(Box::new(turn_on));
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
        if only == Some(Step::AnotherProvider) {
            self.adding = true;
            self.provider(&mut config, &secrets, language).await?;
        }
        // Where the Conversation is held: in a browser, in Telegram, or both.
        let linked = config.telegram_owner.is_some() && has(TELEGRAM_TOKEN)?;
        let (in_a_browser, in_telegram) = match only {
            None if config.interface.is_none() && !linked => self.place(language)?,
            None => (false, false),
            Some(only) => (only == Step::Interface, only == Step::Telegram),
        };
        if in_a_browser {
            self.interface(&mut config, language)?;
        }
        if in_telegram {
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
            "\nAvec quelle IA ton agent doit-il réfléchir ?",
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

        if known.speaks == Speaks::ChatGptPlan {
            self.continue_with_chatgpt(&root, language).await?;
        }
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
                // What trying it out showed, when it was tried.
                let known = crate::rating(&self.home, known.name, model)
                    .map(|verdict| format!(": {}", verdict.in_brief(language)))
                    .unwrap_or_default();
                self.say(&format!("  {}. {model}{known}", place + 1))?;
            }
        }
        // What real tryouts say of this Provider's models, to choose by.
        if let Some(advice) = crate::advice(&self.home, known.name, language) {
            self.say(&format!("\n{advice}"))?;
        }
        let model = loop {
            let answer =
                self.ask(language.say("Model (number or name) > ", "Modèle (numéro ou nom) > "))?;
            let model = answer
                .parse::<usize>()
                .ok()
                .and_then(|number| offered.get(number.wrapping_sub(1)).cloned())
                .unwrap_or(answer);
            if let Err(error) =
                self.check(&known, key.as_deref().unwrap_or_default(), &model, &root).await
            {
                self.say(&format!(
                    "{} {}",
                    language.say("That did not work:", "Cela n'a pas fonctionné :"),
                    crate::agent::explain(&error, language)
                ))?;
                continue;
            }
            // A model nobody rated is tried out on a few real tasks, so that
            // its Owner knows what to expect before relying on it.
            let verdict = match crate::rating(&self.home, known.name, &model) {
                Some(verdict) => Some(verdict),
                None => {
                    self.say(language.say(
                        "Trying this model out on a few real tasks (about a minute)...",
                        "Essai de ce modèle sur quelques vraies tâches (environ une minute)...",
                    ))?;
                    let tried =
                        connect_in(&self.home, known.name, key.as_deref(), &model, Some(&root))
                            .map_err(ProviderError::Failed);
                    let tried = match tried {
                        Ok(provider) => crate::try_out(&provider).await,
                        Err(error) => Err(error),
                    };
                    match tried {
                        Ok(verdict) => {
                            let _ = crate::keep_rating(&self.home, known.name, &model, &verdict);
                            Some(verdict)
                        }
                        // The model answers; it will be tried another time.
                        Err(error) => {
                            self.say(&format!(
                                "{} {}",
                                language.say(
                                    "The tryout could not be finished:",
                                    "L'essai n'a pas pu aller au bout :"
                                ),
                                crate::agent::explain(&error, language)
                            ))?;
                            None
                        }
                    }
                }
            };
            let Some(verdict) = verdict else { break model };
            self.say(&verdict.told(language))?;
            if !verdict.strongly_discouraged() {
                break model;
            }
            // Nothing is blocked, but this choice is made knowingly.
            let insisted = self.ask(language.say(
                "An Agent on this model can only talk. Choose it anyway? Type yes, or anything else to choose another > ",
                "Un agent sur ce modèle ne peut que parler. Le choisir quand même ? Tape oui, ou autre chose pour en choisir un autre > ",
            ))?;
            if matches!(insisted.trim().to_lowercase().as_str(), "yes" | "oui") {
                break model;
            }
        };

        let mut chosen = ProviderChoice {
            name: known.name.into(),
            model: model.clone(),
            address,
            key: None,
            daily_tokens: None,
        };
        // With several Providers each keeps a key of its own.
        if self.adding || !config.fallbacks.is_empty() {
            crate::providers::give_each_its_key(config, secrets).map_err(stopped)?;
            if let Some(key) = &key {
                let named = crate::providers::key_of(known.name);
                secrets.set(&named, key).map_err(stopped)?;
                chosen.key = Some(named);
            }
        } else if let Some(key) = &key {
            secrets.set(PROVIDER_KEY, key).map_err(stopped)?;
        }
        if self.adding {
            // Last in line, in place of one of the same name.
            let mut all = config.providers();
            let place = match all.iter().position(|provider| provider.name == chosen.name) {
                Some(at) => {
                    chosen.daily_tokens = all[at].daily_tokens;
                    all[at] = chosen;
                    at + 1
                }
                None => {
                    all.push(chosen);
                    all.len()
                }
            };
            config.set_providers(all);
            config.save(&self.home).map_err(stopped)?;
            return self.say(&format!(
                "{} {} {place}. {}",
                language.say("The AI answers. ✓", "L'IA répond. ✓"),
                language.say(
                    "It takes over when those before it cannot answer, at place",
                    "Elle prend le relais quand celles d'avant ne peuvent pas répondre, à la place",
                ),
                language.say(
                    "`orbite providers` shows the order and how to change it.",
                    "`orbite providers` montre l'ordre et comment le changer.",
                ),
            ));
        }
        // Whatever takes over from the one replaced stays in line.
        let after: Vec<ProviderChoice> =
            config.fallbacks.iter().filter(|other| other.name != chosen.name).cloned().collect();
        config.provider = Some(chosen);
        config.fallbacks = after;
        config.light_model = None;
        config.save(&self.home).map_err(stopped)?;
        self.say(language.say("The AI answers. ✓", "L'IA répond. ✓"))?;

        // Summaries and wake-ups do not need the best model. What was chosen
        // so far is kept even if setup is closed at this question.
        let Ok(answer) = self.ask_or_pass(language.say(
            "A lighter model for simple work, such as summaries and wake-ups (number or name; Enter for none) > ",
            "Un modèle plus léger pour le travail simple, comme les résumés et les réveils (numéro ou nom ; Entrée pour aucun) > ",
        )) else {
            return Ok(());
        };
        let light = answer
            .parse::<usize>()
            .ok()
            .and_then(|number| offered.get(number.wrapping_sub(1)).cloned())
            .unwrap_or(answer);
        if light.trim().is_empty() || light == model {
            return Ok(());
        }
        match self.check(&known, key.as_deref().unwrap_or_default(), &light, &root).await {
            Ok(()) => {
                config.light_model = Some(light);
                config.save(&self.home).map_err(stopped)
            }
            Err(error) => self.say(&format!(
                "{} {}",
                language.say(
                    "That one did not work, so none is used:",
                    "Celui-ci n'a pas fonctionné, aucun n'est donc utilisé :"
                ),
                crate::agent::explain(&error, language)
            )),
        }
    }

    /// Signs the Owner in to their ChatGPT plan: a link to follow in any
    /// browser, then the address that browser ended on, pasted here.
    async fn continue_with_chatgpt(
        &mut self,
        root: &str,
        language: Language,
    ) -> Result<(), SetupError> {
        let Ok(crate::Connected::ChatGpt(chatgpt)) =
            connect_in(&self.home, "chatgpt", None, "", Some(root))
        else {
            return Err(SetupError("The ChatGPT plan could not be prepared.".into()));
        };
        self.say(language.say(
            "\nYour agent will think with the usage included in your ChatGPT plan (an eligible \
             Plus or Pro plan), through OpenAI's sign-in for open-source apps. That programme is \
             a preview: OpenAI may change it or end it.",
            "\nTon agent réfléchira avec l'utilisation comprise dans ton forfait ChatGPT \
             (un forfait Plus ou Pro éligible), par la connexion qu'OpenAI ouvre aux applications \
             libres. Ce programme est un aperçu : OpenAI peut le modifier ou l'arrêter.",
        ))?;
        let account = loop {
            let begun = chatgpt.sign_in().map_err(stopped)?;
            self.say(&format!(
                "\n{}\n  {}\n{}",
                language.say(
                    "Continue with ChatGPT: open this link in a browser, on any device, and sign in.",
                    "Continuer avec ChatGPT : ouvre ce lien dans un navigateur, sur n'importe \
                     quel appareil, et connecte-toi.",
                ),
                begun.link(),
                language.say(
                    "Your browser then ends on a page that does not load, at an address starting \
                     with http://127.0.0.1:1455 . That is expected: copy that whole address and \
                     paste it here.",
                    "Ton navigateur finit alors sur une page qui ne s'affiche pas, à une adresse \
                     commençant par http://127.0.0.1:1455 . C'est normal : copie cette adresse \
                     entière et colle-la ici.",
                ),
            ))?;
            let ended_on = self.ask(language.say("Address > ", "Adresse > "))?;
            match chatgpt.signed_in_at(&begun, &ended_on).await {
                Ok(account) => break account,
                Err(reason) => self.say(&format!(
                    "{} {reason}.",
                    language.say("That did not work:", "Cela n'a pas fonctionné :")
                ))?,
            }
        };
        self.say(&format!(
            "{}{} ✓\n{} {}",
            language.say("You're using your ChatGPT plan", "Tu utilises ton forfait ChatGPT"),
            if account.is_empty() { String::new() } else { format!(" ({account})") },
            language.say("Manage usage:", "Gérer l'utilisation :"),
            crate::chatgpt::MANAGE_USAGE,
        ))
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
        let provider = connect_in(&self.home, known.name, Some(key), model, Some(root))
            .map_err(ProviderError::Failed)?;
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
        if known.speaks == Speaks::ChatGptPlan {
            let Ok(crate::Connected::ChatGpt(chatgpt)) =
                connect_in(&self.home, known.name, None, "", Some(root))
            else {
                return Vec::new();
            };
            return chatgpt.models().await.into_iter().map(|(slug, _)| slug).take(15).collect();
        }
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

    /// Asks where the Owner will talk to their Agent: in a browser, in
    /// Telegram, in that order.
    fn place(&mut self, language: Language) -> Result<(bool, bool), SetupError> {
        self.say(language.say(
            "\nWhere will you talk to your agent?\n  \
             1. In a browser, on your computer and your phone\n  \
             2. In Telegram\n  \
             3. Both",
            "\nOù parleras-tu à ton agent ?\n  \
             1. Dans un navigateur, sur ton ordinateur et ton téléphone\n  \
             2. Dans Telegram\n  \
             3. Les deux",
        ))?;
        loop {
            let answer = self.ask(language.say("Number > ", "Numéro > "))?.to_lowercase();
            match answer.as_str() {
                "1" | "browser" | "web" | "navigateur" => return Ok((true, false)),
                "2" | "telegram" => return Ok((false, true)),
                "3" | "both" | "les deux" => return Ok((true, true)),
                _ => {}
            }
        }
    }

    /// Turns the web interface on, and says where it will be reached.
    fn interface(&mut self, config: &mut Config, language: Language) -> Result<(), SetupError> {
        match &mut self.turn_interface_on {
            Some(turn_on) => {
                turn_on().map_err(stopped)?;
                *config = Config::load(&self.home).map_err(stopped)?;
            }
            None => {
                config.interface = Some("127.0.0.1:8377".into());
                config.save(&self.home).map_err(stopped)?;
            }
        }
        let said = match (&config.interface_reached_at, &config.interface) {
            (Some(reached), _) => format!(
                "{}\n  {reached}\n{}",
                language.say(
                    "\nYour agent will be reached in a browser at:",
                    "\nTon agent sera joignable dans un navigateur à :"
                ),
                language.say(
                    "from the devices of your Tailscale network, encrypted. Nobody else reaches it. ✓",
                    "depuis les appareils de ton réseau Tailscale, en chiffré. Personne d'autre n'y accède. ✓"
                ),
            ),
            (None, served) => format!(
                "{}\n  http://{}\n{}",
                language.say(
                    "\nYour agent will be reached in a browser on this machine, at:",
                    "\nTon agent sera joignable dans un navigateur sur cette machine, à :"
                ),
                served.as_deref().unwrap_or_default(),
                language.say(
                    "To reach it from your phone too, put this machine and your phone on Tailscale \
                     (tailscale.com), then run: sudo orbite interface on",
                    "Pour y accéder aussi depuis ton téléphone, mets cette machine et ton téléphone \
                     sur Tailscale (tailscale.com), puis lance : sudo orbite interface on"
                ),
            ),
        };
        self.say(&said)
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
            "\nTu parleras à ton agent dans Telegram, par un bot qui t'appartient :\n  \
             1. Dans Telegram, ouvre une discussion avec @BotFather.\n  \
             2. Envoie /newbot et réponds à ses deux questions (un nom, puis un identifiant finissant par « bot »).\n  \
             3. BotFather te donne un jeton : colle-le ici.",
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
                        "Telegram ne connaît pas ce jeton. Recopie-le depuis @BotFather",
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
                "Ouvre maintenant sa discussion dans Telegram et envoie-lui ce code :",
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
                            "Aucun code n'est arrivé. Relance la configuration quand tu seras prêt à l'envoyer.",
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
            "Lié : désormais je t'écoute ici, et personne d'autre.",
        );
        let _ = self.bot(&token, "sendMessage", json!({ "chat_id": owner, "text": linked })).await;
        self.say(
            language.say("Your Telegram account is linked. ✓", "Ton compte Telegram est lié. ✓"),
        )
    }

    fn name(&mut self, config: &mut Config, language: Language) -> Result<(), SetupError> {
        self.say(language.say(
            "\nWhat will you call your agent? Some ideas:",
            "\nComment appelleras-tu ton agent ? Quelques idées :",
        ))?;
        for (place, name) in NAMES.iter().enumerate() {
            self.say(&format!("  {}. {name}", place + 1))?;
        }
        let name = loop {
            let answer = self.ask(
                language.say("Number, or a name of your own > ", "Numéro, ou un nom à toi > "),
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
        let telegram = match (config.telegram_owner, secrets.get(TELEGRAM_TOKEN).map_err(stopped)?)
        {
            (Some(owner), Some(token)) => Some((owner, token)),
            _ => None,
        };
        if telegram.is_none() && config.interface.is_none() {
            return Ok(());
        }
        let greeting = if language == Language::French {
            format!(
                "Bonjour, je suis {name}, ton agent. Je vis sur ta machine, je me souviens de ce \
                 qui compte et je te demande avant tout ce qui change quelque chose. Dis-moi ce \
                 dont tu as besoin."
            )
        } else {
            format!(
                "Hello, I'm {name}, your agent. I live on your machine, remember what matters, and \
                 ask before anything that changes something. Tell me what you need."
            )
        };
        if let Some((owner, token)) = &telegram {
            let _ =
                self.bot(token, "sendMessage", json!({ "chat_id": owner, "text": greeting })).await;
        }
        let Some(served) = &config.interface else {
            return self.say(&format!(
                "\n{} {name} {}",
                language.say("All set.", "Tout est prêt."),
                language.say("is waiting for you in Telegram.", "t'attend dans Telegram.")
            ));
        };
        // In a browser: greeted there too, and let in by a link that
        // carries the code of a first device.
        crate::interface::greet(&self.home, &greeting).map_err(stopped)?;
        let code = crate::pairing_code(&self.home).map_err(stopped)?;
        let at = config.interface_reached_at.clone().unwrap_or(format!("http://{served}"));
        self.say(&format!(
            "\n{} {name} {}\n\n  {at}/#{code}\n\n{}{}",
            language.say("All set.", "Tout est prêt."),
            language.say(
                "is waiting for you. Open this on your phone or your computer:",
                "t'attend. Ouvre ceci sur ton téléphone ou ton ordinateur :"
            ),
            language.say(
                "The link lets one device in, for ten minutes. For another, ask your agent to add a device.",
                "Le lien autorise un appareil, pendant dix minutes. Pour un autre, demande à ton agent d'ajouter un appareil."
            ),
            if telegram.is_some() {
                language.say("\nIt is in Telegram too.", "\nIl est aussi dans Telegram.")
            } else {
                ""
            },
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
    /// Asks a question that may be passed over with Enter: the answer is
    /// then empty.
    fn ask_or_pass(&mut self, question: &str) -> Result<String, SetupError> {
        write!(self.shown, "{question}").and_then(|()| self.shown.flush()).map_err(stopped)?;
        let mut line = String::new();
        if self.typed.read_line(&mut line).map_err(stopped)? == 0 {
            return Err(SetupError("Setup was cut short; run it again to go on from here.".into()));
        }
        Ok(line.trim().to_owned())
    }

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
