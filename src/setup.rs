//! The guided setup: a few questions in the terminal, each answer checked
//! for real before the next, and everything kept as it goes so that a setup
//! cut short resumes where it stopped.

use std::fmt;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use crate::front::{Choice, Front, Left, Lines, Picked};
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

/// `said` without `key` in it, were whoever was asked to say it back.
fn hidden(said: &str, key: &str) -> String {
    if key.is_empty() { said.to_owned() } else { said.replace(key, "…") }
}

/// Turns the web interface on for the installation, as whoever may: where
/// Orbite is installed that takes more than the Agent's account.
type TurnOn = Box<dyn FnMut() -> Result<(), String> + Send>;

impl From<Left> for SetupError {
    fn from(_: Left) -> Self {
        SetupError("Setup was cut short; run it again to go on from here.".into())
    }
}

pub struct Setup<F> {
    home: PathBuf,
    /// What the questions are asked through.
    front: F,
    turn_interface_on: Option<TurnOn>,
    /// True when this machine is on a Tailscale network, which can serve
    /// the web interface to the Owner's other devices.
    tailscale_here: bool,
    /// The address this Host carries on the Internet, when it has one.
    public_address: Option<std::net::IpAddr>,
    providers_at: Option<String>,
    /// True when the Provider being set up is added to those there are.
    adding: bool,
    telegram_at: String,
    code: Option<String>,
    look_every: Duration,
    give_up_after: Duration,
    http: reqwest::Client,
}

impl<R: BufRead + Send, W: Write + Send> Setup<Lines<R, W>> {
    /// Setup of the installation whose folder is `home`, asking in `shown`
    /// and reading answers from `typed`, line after line.
    pub fn in_home(home: impl Into<PathBuf>, typed: R, shown: W) -> Self {
        Self::shown_with(home, Lines::new(typed, shown))
    }

    /// Reads keys and tokens with `read`, which shows nothing of what is typed.
    pub fn reading_secrets_with(
        mut self,
        read: impl FnMut(&str) -> std::io::Result<String> + Send + 'static,
    ) -> Self {
        self.front.reading_secrets_with(Box::new(read));
        self
    }
}

impl<F: Front> Setup<F> {
    /// Setup of the installation whose folder is `home`, asked through `front`.
    pub fn shown_with(home: impl Into<PathBuf>, front: F) -> Self {
        Self {
            home: home.into(),
            front,
            turn_interface_on: None,
            tailscale_here: false,
            public_address: None,
            providers_at: None,
            adding: false,
            telegram_at: "https://api.telegram.org".into(),
            code: None,
            look_every: Duration::from_secs(2),
            give_up_after: Duration::from_secs(600),
            http: reqwest::Client::new(),
        }
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

    /// Says whether this machine is on a Tailscale network: the Owner is
    /// then asked before the web interface is served through it.
    pub fn where_tailscale_is(mut self, here: bool) -> Self {
        self.tailscale_here = here;
        self
    }

    /// Says which address of the Internet this machine carries, if any.
    pub fn where_the_public_address_is(mut self, address: Option<std::net::IpAddr>) -> Self {
        self.public_address = address;
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
        // How far along the Owner is, of the four things a first setup asks.
        const STEPS: usize = 4;

        if wanted(Step::Language, config.language.is_some()) {
            self.front.step(1, STEPS);
            self.language(&mut config)?;
        }
        let language = Language::of(config.language.as_deref());
        self.front.speak(language);
        if wanted(Step::Provider, config.provider.is_some()) {
            self.front.step(2, STEPS);
            self.provider(&mut config, &secrets, language).await?;
        }
        if only == Some(Step::AnotherProvider) {
            self.adding = true;
            self.provider(&mut config, &secrets, language).await?;
        }
        // Where the Conversation is held: in a browser, in Telegram, or both.
        let linked = config.telegram_owner.is_some() && has(TELEGRAM_TOKEN)?;
        let (in_a_browser, in_telegram) = match only {
            None if config.interface.is_none() && !linked => {
                self.front.step(3, STEPS);
                self.place(language)?
            }
            None => (false, false),
            Some(only) => (only == Step::Interface, only == Step::Telegram),
        };
        if in_a_browser {
            self.interface(&mut config, language, only.is_some())?;
        }
        if in_telegram {
            self.telegram(&mut config, &secrets, language).await?;
        }
        if wanted(Step::Name, config.agent_name.is_some()) {
            self.front.step(4, STEPS);
            self.name(&mut config, language)?;
        }
        if only.is_none() {
            self.greet(&config, &secrets, language).await?;
        }
        Ok(())
    }

    fn language(&mut self, config: &mut Config) -> Result<(), SetupError> {
        // The one the machine was set to comes first, when it is one of ours.
        let of_the_machine = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .unwrap_or_default();
        let suggested = usize::from(of_the_machine.to_lowercase().starts_with("fr"));
        let choices =
            [Choice::of("English").also(&["en"]), Choice::of("Français").also(&["fr", "francais"])];
        let chosen = match self.front.choose("Language / Langue", &choices, suggested, None)? {
            Picked::Choice(1) => "fr",
            _ => "en",
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
        let mut choices: Vec<Choice> = known_providers()
            .iter()
            .map(|known| {
                // A name, and what a label says more in brackets.
                let (label, hint) = match known.label.split_once(" (") {
                    Some((label, hint)) => (label, hint.trim_end_matches(')')),
                    None => (known.label, ""),
                };
                let hint = match (language, hint) {
                    (Language::French, "Plus or Pro, no API key") => {
                        "forfait Plus ou Pro, sans clé"
                    }
                    (Language::French, "many models, one key") => "beaucoup de modèles, une clé",
                    (Language::French, "on this machine") => "sur cette machine",
                    (_, hint) => hint,
                };
                Choice::of(label).hinted(hint).also(&[known.name])
            })
            .collect();
        choices.push(
            Choice::of(language.say("Another server", "Un autre serveur"))
                .hinted(language.say("speaking OpenAI's protocol", "parlant le protocole d'OpenAI"))
                .also(&["custom"]),
        );
        let question = language.say(
            "Which AI should your agent think with?",
            "Avec quelle IA ton agent doit-il réfléchir ?",
        );
        let known = match self.front.choose(question, &choices, 0, None)? {
            Picked::Choice(place) if place < known_providers().len() => known_providers()[place],
            _ => KnownProvider {
                name: "custom",
                label: "custom",
                speaks: Speaks::OpenAi,
                api_root: "",
                needs_key: true,
            },
        };
        let address =
            match (&self.providers_at, known.name) {
                (Some(address), _) => Some(address.clone()),
                (None, "custom") => Some(self.front.ask(
                    language.say("Its address (https://…/v1)", "Son adresse (https://…/v1)"),
                )?),
                (None, _) => None,
            };
        let root = address.clone().unwrap_or_else(|| known.api_root.to_owned());
        let called = known.label.split(" (").next().unwrap_or(known.label);

        if known.speaks == Speaks::ChatGptPlan {
            self.continue_with_chatgpt(&root, language).await?;
        }
        // The key first, seen to be accepted, then the model.
        let key = loop {
            if !known.needs_key {
                break None;
            }
            let key = self.front.ask_hidden(&match (language, known.name) {
                (Language::French, "custom") => "Sa clé d'API".to_owned(),
                (Language::English, "custom") => "Its API key".to_owned(),
                (Language::French, _) => format!("Ta clé d'API {called}"),
                (Language::English, _) => format!("Your {called} API key"),
            })?;
            self.front.working(language.say("Reaching it...", "Connexion..."));
            match self.check(&known, &key, "", &root).await {
                // Without the key itself, were the Provider to say it back.
                Err(ProviderError::Rejected(reason)) => self.front.wrong(&format!(
                    "{} {}",
                    language.say(
                        "The provider refused this key:",
                        "Le fournisseur a refusé cette clé :"
                    ),
                    reason.replace(&key, "…")
                )),
                Err(ProviderError::Unreachable(reason)) => self.front.wrong(&format!(
                    "{} {reason}",
                    language.say(
                        "The provider could not be reached:",
                        "Le fournisseur est injoignable :"
                    )
                )),
                // Anything else is about the model, which is not chosen yet.
                _ => break Some(key),
            }
        };
        let key_of = key.as_deref().unwrap_or_default().to_owned();

        // The model is not a question: the one this Provider is known to
        // serve an Agent well with is taken, when it answers. The Owner
        // chooses another later by asking their Agent.
        let (usual, lighter) = crate::catalogue::usual_models(known.name);
        let mut model = None;
        let mut offered = Vec::new();
        if known.speaks == Speaks::ChatGptPlan {
            // A plan says itself which models it comes with: the usual one
            // when it is among them, or the first it names.
            offered = self.models(&known, None, &root).await;
            let of_the_plan = usual
                .iter()
                .map(|usual| (*usual).to_owned())
                .filter(|usual| offered.contains(usual))
                .chain(offered.first().cloned());
            for candidate in of_the_plan {
                self.front.working(language.say("Reaching it...", "Connexion..."));
                if self.check(&known, "", &candidate, &root).await.is_ok() {
                    model = Some(candidate);
                    break;
                }
            }
        }
        let by_key = known.speaks != Speaks::ChatGptPlan;
        for candidate in usual.iter().filter(|_| model.is_none() && by_key) {
            self.front.working(language.say("Reaching it...", "Connexion..."));
            if self.check(&known, &key_of, candidate, &root).await.is_ok() {
                model = Some((*candidate).to_owned());
                break;
            }
        }
        if model.is_none() {
            self.front.working(language.say("Reaching it...", "Connexion..."));
            offered = self.models(&known, key.as_deref(), &root).await;
            // A Provider with one model to its name has nothing to choose.
            if let [only] = &offered[..]
                && self.check(&known, &key_of, only, &root).await.is_ok()
            {
                model = Some(only.clone());
            }
        }
        let model = match model {
            Some(model) => model,
            None => self.model(&known, &key_of, &root, &offered, language).await?,
        };
        // Summaries and wake-ups do not need the best model.
        let mut light = None;
        for candidate in lighter.iter().filter(|candidate| **candidate != model) {
            if self.check(&known, &key_of, candidate, &root).await.is_ok() {
                light = Some((*candidate).to_owned());
                break;
            }
        }

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
        let answers = match language {
            Language::French => format!("{called} répond ({model})"),
            Language::English => format!("{called} answers ({model})"),
        };
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
            self.front.good(&answers);
            self.front.say(&format!(
                "{} {place}. {}",
                language.say(
                    "It takes over when those before it cannot answer, at place",
                    "Elle prend le relais quand celles d'avant ne peuvent pas répondre, à la place",
                ),
                language.say(
                    "`orbite providers` shows the order and how to change it.",
                    "`orbite providers` montre l'ordre et comment le changer.",
                ),
            ));
            return Ok(());
        }
        // Whatever takes over from the one replaced stays in line.
        let after: Vec<ProviderChoice> =
            config.fallbacks.iter().filter(|other| other.name != chosen.name).cloned().collect();
        config.provider = Some(chosen);
        config.fallbacks = after;
        config.light_model = light;
        config.save(&self.home).map_err(stopped)?;
        self.front.good(&answers);
        Ok(())
    }

    /// Asks which model to think with, where the Provider has no usual one:
    /// one it offers, or one typed by name.
    async fn model(
        &mut self,
        known: &KnownProvider,
        key: &str,
        root: &str,
        offered: &[String],
        language: Language,
    ) -> Result<String, SetupError> {
        let choices: Vec<Choice> = offered
            .iter()
            .map(|model| {
                // What trying it out showed, when it was tried.
                let rated = crate::rating(&self.home, known.name, model)
                    .map(|verdict| verdict.in_brief(language))
                    .unwrap_or_default();
                Choice::of(model.as_str()).hinted(rated)
            })
            .collect();
        loop {
            let question = language.say("Which model?", "Quel modèle ?");
            let own = language.say("or the name of another", "ou le nom d'un autre");
            let model = match self.front.choose(question, &choices, 0, Some(own))? {
                Picked::Choice(place) => offered[place].clone(),
                Picked::Typed(name) => name,
            };
            self.front.working(language.say("Reaching it...", "Connexion..."));
            if let Err(error) = self.check(known, key, &model, root).await {
                self.front.wrong(&format!(
                    "{} {}",
                    language.say("That did not work:", "Cela n'a pas fonctionné :"),
                    hidden(&crate::agent::explain(&error, language), key)
                ));
                continue;
            }
            // What is known of it is said; nothing is tried here.
            let Some(verdict) = crate::rating(&self.home, known.name, &model) else {
                return Ok(model);
            };
            if !verdict.strongly_discouraged() {
                return Ok(model);
            }
            // Nothing is blocked, but this choice is made knowingly.
            self.front.say(&verdict.told(language));
            let insist = [
                Choice::of(language.say("Choose another", "En choisir un autre"))
                    .also(&["no", "non"]),
                Choice::of(language.say("Take it anyway", "Le prendre quand même"))
                    .also(&["yes", "oui"]),
            ];
            let asked = language.say(
                "An Agent on this model can only talk.",
                "Un agent sur ce modèle ne peut que parler.",
            );
            if self.front.choose(asked, &insist, 0, None)? == Picked::Choice(1) {
                return Ok(model);
            }
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
        let account = loop {
            let begun = chatgpt.sign_in().map_err(stopped)?;
            self.front.say(language.say(
                "Your agent will think with the usage included in your ChatGPT plan (an eligible \
                 Plus or Pro plan), through OpenAI's sign-in for open-source apps. That programme is \
                 a preview: OpenAI may change it or end it.\n\n\
                 Continue with ChatGPT: open this link in a browser, on any device, and sign in.",
                "Ton agent réfléchira avec l'utilisation comprise dans ton forfait ChatGPT \
                 (un forfait Plus ou Pro éligible), par la connexion qu'OpenAI ouvre aux applications \
                 libres. Ce programme est un aperçu : OpenAI peut le modifier ou l'arrêter.\n\n\
                 Continuer avec ChatGPT : ouvre ce lien dans un navigateur, sur n'importe \
                 quel appareil, et connecte-toi.",
            ));
            self.front.show(begun.link());
            self.front.say(language.say(
                "Your browser then ends on a page that does not load, at an address starting \
                 with http://127.0.0.1:1455 . That is expected: copy that whole address and \
                 paste it here.",
                "Ton navigateur finit alors sur une page qui ne s'affiche pas, à une adresse \
                 commençant par http://127.0.0.1:1455 . C'est normal : copie cette adresse \
                 entière et colle-la ici.",
            ));
            let ended_on = self.front.ask(language.say("Address", "Adresse"))?;
            self.front.working(language.say("Signing in...", "Connexion..."));
            match chatgpt.signed_in_at(&begun, &ended_on).await {
                Ok(account) => break account,
                Err(reason) => self.front.wrong(&format!(
                    "{} {reason}.",
                    language.say("That did not work:", "Cela n'a pas fonctionné :")
                )),
            }
        };
        self.front.good(&format!(
            "{}{}",
            language.say("You're using your ChatGPT plan", "Tu utilises ton forfait ChatGPT"),
            if account.is_empty() { String::new() } else { format!(" ({account})") },
        ));
        self.front.say(&format!(
            "{} {}",
            language.say("Manage usage:", "Gérer l'utilisation :"),
            crate::chatgpt::MANAGE_USAGE,
        ));
        Ok(())
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
        // A Provider that never answers is not waited for without end.
        match tokio::time::timeout(Duration::from_secs(45), provider.reply(request)).await {
            Ok(replied) => replied.map(|_| ()),
            Err(_) => Err(ProviderError::Unreachable("it did not answer in time".into())),
        }
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
        let choices =
            [
                Choice::of(language.say("In a browser", "Dans un navigateur"))
                    .hinted(language.say(
                        "on your computer and your phone",
                        "sur ton ordinateur et ton téléphone",
                    ))
                    .also(&["browser", "web", "navigateur"]),
                Choice::of(language.say("In Telegram", "Dans Telegram")).also(&["telegram"]),
                Choice::of(language.say("Both", "Les deux")).also(&["both", "les deux"]),
            ];
        let question =
            language.say("Where will you talk to your agent?", "Où parleras-tu à ton agent ?");
        Ok(match self.front.choose(question, &choices, 0, None)? {
            Picked::Choice(1) => (false, true),
            Picked::Choice(2) => (true, true),
            _ => (true, false),
        })
    }

    /// Turns the web interface on, reached from the Owner's devices the
    /// way they choose: through their Tailscale network, or at the address
    /// this Host has on the Internet. Done on its own, says where it will
    /// be reached, which a whole setup says at its end.
    fn interface(
        &mut self,
        config: &mut Config,
        language: Language,
        alone: bool,
    ) -> Result<(), SetupError> {
        // Asked where there is a real machine to open: either way changes
        // something of the Owner's, which is theirs to say.
        let mut public = None;
        if self.turn_interface_on.is_some() {
            let on_tailscale = if self.tailscale_here {
                language.say(
                    "already on this machine, set up by itself",
                    "déjà sur cette machine, réglé tout seul",
                )
            } else {
                language.say(
                    "a private network between your devices",
                    "un réseau privé entre tes appareils",
                )
            };
            let choices = [
                Choice::of(language.say("Through Tailscale", "Par Tailscale"))
                    .hinted(on_tailscale)
                    .also(&["tailscale"]),
                Choice::of(language.say(
                    "At this server's public address",
                    "Par l'adresse publique de ce serveur",
                ))
                .hinted(
                    language.say("for a server on the Internet", "pour un serveur sur Internet"),
                )
                .also(&["public"]),
            ];
            // What the machine has comes first.
            let suggested = usize::from(!self.tailscale_here && self.public_address.is_some());
            let question = language.say(
                "How will your devices reach your agent?",
                "Par où tes appareils rejoindront-ils ton agent ?",
            );
            if self.front.choose(question, &choices, suggested, None)? == Picked::Choice(1) {
                public = Some(self.public(language)?);
            }
        }
        self.front.working(language.say("Opening the page...", "Ouverture de la page..."));
        match (public, &mut self.turn_interface_on) {
            (Some(address), _) => {
                let named = match address {
                    std::net::IpAddr::V4(address) => address.to_string(),
                    std::net::IpAddr::V6(address) => format!("[{address}]"),
                };
                config.interface = Some("127.0.0.1:8377".into());
                config.interface_reached_at = Some(format!("https://{named}"));
                config.interface_public = Some(address.to_string());
                config.save(&self.home).map_err(stopped)?;
            }
            (None, Some(turn_on)) if self.tailscale_here => {
                turn_on().map_err(stopped)?;
                *config = Config::load(&self.home).map_err(stopped)?;
            }
            // Nothing to reach it through yet: on this machine meanwhile.
            (None, _) => {
                config.interface = Some("127.0.0.1:8377".into());
                config.save(&self.home).map_err(stopped)?;
            }
        }
        match (&config.interface_public, &config.interface_reached_at, &config.interface) {
            (Some(_), reached, _) => {
                self.front.good(language.say(
                    "Reached at this server's address, encrypted",
                    "Joignable à l'adresse de ce serveur, en chiffré",
                ));
                if alone {
                    self.front.show(reached.as_deref().unwrap_or_default());
                    self.front.say(language.say(
                        "Ports 80 and 443 of this server must be open to the Internet.",
                        "Les ports 80 et 443 de ce serveur doivent être ouverts vers Internet.",
                    ));
                }
            }
            (None, Some(reached), _) => {
                self.front.good(language.say(
                    "Reached from the devices of your Tailscale network, encrypted",
                    "Joignable depuis les appareils de ton réseau Tailscale, en chiffré",
                ));
                if alone {
                    self.front.show(reached);
                }
            }
            (None, None, served) => {
                self.front.good(language.say(
                    "Reached in a browser on this machine, for now",
                    "Joignable dans un navigateur sur cette machine, pour l'instant",
                ));
                if alone {
                    self.front.show(&format!("http://{}", served.as_deref().unwrap_or_default()));
                    self.front.say(self.to_reach_it_from_elsewhere(language));
                }
            }
        }
        Ok(())
    }

    /// What to do for the page to be reached from other devices, where
    /// Tailscale is not on this machine yet.
    fn to_reach_it_from_elsewhere(&self, language: Language) -> &'static str {
        language.say(
            "To reach it from your phone, put Tailscale on this machine and on your phone \
             (tailscale.com/download), then tell your agent: turn the web interface on.",
            "Pour y accéder depuis ton téléphone, mets Tailscale sur cette machine et sur ton \
             téléphone (tailscale.com/download), puis dis à ton agent : active l'interface web.",
        )
    }

    /// The address this Host has on the Internet: the one it carries, or
    /// the one its Owner gives when it sits behind another's.
    fn public(&mut self, language: Language) -> Result<std::net::IpAddr, SetupError> {
        if let Some(address) = self.public_address {
            return Ok(address);
        }
        loop {
            let answer = self.front.ask(language.say(
                "This server's public address (the one you connect to it by)",
                "L'adresse publique de ce serveur (celle par laquelle tu t'y connectes)",
            ))?;
            match answer.trim().parse::<std::net::IpAddr>() {
                Ok(address) if !crate::is_private(&std::net::SocketAddr::new(address, 0)) => {
                    return Ok(address);
                }
                Ok(_) => self.front.wrong(language.say(
                    "That is an address of a private network: nothing on the Internet reaches it.",
                    "C'est une adresse de réseau privé : rien sur Internet ne l'atteint.",
                )),
                Err(_) => self.front.wrong(language.say(
                    "That is not an address: it looks like 203.0.113.7.",
                    "Ce n'est pas une adresse : elle ressemble à 203.0.113.7.",
                )),
            }
        }
    }

    async fn telegram(
        &mut self,
        config: &mut Config,
        secrets: &Secrets,
        language: Language,
    ) -> Result<(), SetupError> {
        self.front.say(language.say(
            "You will talk to your agent in Telegram, through a bot of your own:\n  \
             1. In Telegram, open a chat with @BotFather.\n  \
             2. Send /newbot and follow its two questions (a name, then a username ending in \"bot\").\n  \
             3. BotFather gives you a token: paste it here.",
            "Tu parleras à ton agent dans Telegram, par un bot qui t'appartient :\n  \
             1. Dans Telegram, ouvre une discussion avec @BotFather.\n  \
             2. Envoie /newbot et réponds à ses deux questions (un nom, puis un identifiant finissant par « bot »).\n  \
             3. BotFather te donne un jeton : colle-le ici.",
        ));
        let (token, username) = loop {
            let token = self.front.ask_hidden(language.say("Bot token", "Jeton du bot"))?;
            self.front
                .working(language.say("Asking Telegram...", "Vérification auprès de Telegram..."));
            match self.bot(&token, "getMe", json!({})).await {
                Ok(bot) => {
                    break (token, bot["username"].as_str().unwrap_or("your bot").to_owned());
                }
                Err(reason) => self.front.wrong(&format!(
                    "{} ({reason})",
                    language.say(
                        "Telegram does not know this token. Copy it again from @BotFather",
                        "Telegram ne connaît pas ce jeton. Recopie-le depuis @BotFather",
                    )
                )),
            }
        };
        secrets.set(TELEGRAM_TOKEN, &token).map_err(stopped)?;

        // Whoever sends the code is the Owner: no identifier to look up or type.
        let code = self.code.clone().unwrap_or_else(|| {
            let mut bytes = [0u8; 4];
            let _ = getrandom::fill(&mut bytes);
            format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000)
        });
        self.front
            .good(&format!("{} @{username}", language.say("Bot recognised:", "Bot reconnu :")));
        self.front.say(language.say(
            "Now open its chat in Telegram and send it this code:",
            "Ouvre maintenant sa discussion dans Telegram et envoie-lui ce code :",
        ));
        self.front.show(&code);
        self.front.working(language.say("Waiting for your code...", "En attente de ton code..."));
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
        self.front
            .good(language.say("Your Telegram account is linked", "Ton compte Telegram est lié"));
        Ok(())
    }

    fn name(&mut self, config: &mut Config, language: Language) -> Result<(), SetupError> {
        let choices: Vec<Choice> = NAMES.iter().map(|name| Choice::of(*name)).collect();
        let question =
            language.say("What will you call your agent?", "Comment appelleras-tu ton agent ?");
        let own = language.say("or a name of your own", "ou un nom à toi");
        let name = loop {
            match self.front.choose(question, &choices, 0, Some(own))? {
                Picked::Choice(place) => break NAMES[place].to_owned(),
                Picked::Typed(name)
                    if name.chars().any(char::is_alphabetic) && name.chars().count() <= 40 =>
                {
                    break name;
                }
                Picked::Typed(_) => {}
            }
        };
        config.agent_name = Some(name);
        config.save(&self.home).map_err(stopped)
    }

    /// The Agent's first words to its Owner, wherever they will talk.
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
            self.front.finish(
                &format!(
                    "{} {name} {}",
                    language.say("All set.", "Tout est prêt."),
                    language.say("is waiting for you in Telegram.", "t'attend dans Telegram.")
                ),
                None,
                "",
            );
            return Ok(());
        };
        // In a browser: greeted there too, and let in by a link that
        // carries the code of a first device.
        crate::interface::greet(&self.home, &greeting).map_err(stopped)?;
        let code = crate::pairing_code(&self.home).map_err(stopped)?;
        let at = config.interface_reached_at.clone().unwrap_or(format!("http://{served}"));
        // Reached from other devices, or from this machine only for now.
        let on_other_devices = language.say(
            "is waiting for you. Open this on your phone or your computer:",
            "t'attend. Ouvre ceci sur ton téléphone ou ton ordinateur :",
        );
        let (open_it, more) = match (&config.interface_public, &config.interface_reached_at) {
            (Some(_), _) => (
                on_other_devices,
                language.say(
                    "\nPorts 80 and 443 of this server must be open to the Internet. The page \
                     answers a few seconds after Orbite starts, once its certificate is given.",
                    "\nLes ports 80 et 443 de ce serveur doivent être ouverts vers Internet. La \
                     page répond quelques secondes après le démarrage d'Orbite, le temps que \
                     son certificat soit donné.",
                )
                .to_owned(),
            ),
            (None, Some(_)) => (on_other_devices, String::new()),
            (None, None) => (
                language.say(
                    "is waiting for you. Open this in a browser on this machine:",
                    "t'attend. Ouvre ceci dans un navigateur sur cette machine :",
                ),
                format!("\n{}", self.to_reach_it_from_elsewhere(language)),
            ),
        };
        self.front.finish(
            &format!("{} {name} {open_it}", language.say("All set.", "Tout est prêt.")),
            Some(&format!("{at}/#{code}")),
            &format!(
                "{}{more}{}",
                language.say(
                    "The link lets one device in, for ten minutes. For another, ask your agent to add a device.",
                    "Le lien autorise un appareil, pendant dix minutes. Pour un autre, demande à ton agent d'ajouter un appareil."
                ),
                if telegram.is_some() {
                    language.say("\nIt is in Telegram too.", "\nIl est aussi dans Telegram.")
                } else {
                    ""
                },
            ),
        );
        Ok(())
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
            // Never with the address asked, which holds the bot's token.
            .map_err(|error| error.without_url().to_string())?;
        let answer: Value =
            response.json().await.map_err(|error| error.without_url().to_string())?;
        if answer["ok"] == true {
            Ok(answer["result"].clone())
        } else {
            Err(answer["description"].as_str().unwrap_or("refused").to_owned())
        }
    }
}
