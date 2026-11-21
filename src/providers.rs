//! The Providers an installation thinks with, in the order they are asked:
//! the first answers while it can, and each next one takes over from there.

use std::path::Path;

use crate::{Config, Language, PROVIDER_KEY, ProviderChoice, Secrets};

impl Config {
    /// Every Provider set up, the preferred one first.
    pub fn providers(&self) -> Vec<ProviderChoice> {
        self.provider.iter().chain(&self.fallbacks).cloned().collect()
    }

    pub(crate) fn set_providers(&mut self, mut all: Vec<ProviderChoice>) {
        self.provider = (!all.is_empty()).then(|| all.remove(0));
        self.fallbacks = all;
    }
}

/// The name of the secret holding the key of the Provider called `name`.
pub(crate) fn key_of(name: &str) -> String {
    let plain: String = name
        .chars()
        .map(
            |letter| if letter.is_ascii_alphanumeric() { letter.to_ascii_uppercase() } else { '_' },
        )
        .collect();
    format!("ORBITE_KEY_{plain}")
}

/// Gives each Provider a key of its own, so that the order can change: the
/// first one set up kept its key under a name that said only "the Provider".
pub(crate) fn give_each_its_key(config: &mut Config, secrets: &Secrets) -> Result<(), String> {
    let Some(first) = config.provider.as_mut().filter(|first| first.key.is_none()) else {
        return Ok(());
    };
    if let Some(key) = secrets.get(PROVIDER_KEY).map_err(|error| error.to_string())? {
        let named = key_of(&first.name);
        secrets.set(&named, &key).map_err(|error| error.to_string())?;
        first.key = Some(named);
        // Under one name only: left there, it would be taken for the key
        // of whichever Provider comes first next and has none of its own.
        secrets.remove(PROVIDER_KEY).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut shown = String::new();
    for (place, digit) in digits.chars().enumerate() {
        if place > 0 && (digits.len() - place).is_multiple_of(3) {
            shown.push(',');
        }
        shown.push(digit);
    }
    shown
}

/// The Providers of the installation in `home`, in order, each with its
/// limit and what it used today when `used` says (by Provider name).
pub fn providers_shown(home: &Path, language: Language, used: &[(String, u64)]) -> String {
    let all = Config::load(home).map(|config| config.providers()).unwrap_or_default();
    if all.is_empty() {
        return language.say("No AI is set up yet.", "Aucune IA n'est configurée.").into();
    }
    let mut shown = String::new();
    for (place, provider) in all.iter().enumerate() {
        let today = used.iter().find(|(by, _)| *by == provider.name).map_or(0, |(_, used)| *used);
        let limit = match provider.daily_tokens {
            Some(limit) => format!(
                "{} {} {} ({} {})",
                language.say("at most", "au plus"),
                grouped(limit),
                language.say("tokens a day", "jetons par jour"),
                grouped(today),
                language.say("used today", "utilisés aujourd'hui"),
            ),
            None => language.say("no limit", "sans limite").to_owned(),
        };
        shown.push_str(&format!(
            "{}. {} · {}: {limit}\n",
            place + 1,
            provider.name,
            provider.model
        ));
    }
    shown.push_str(language.say(
        "The first answers while it can; the next takes over when it is out of usage, over its \
         limit or unreachable.",
        "Le premier répond tant qu'il le peut ; le suivant prend le relais quand il n'a plus \
         d'utilisation, dépasse sa limite ou est injoignable.",
    ));
    shown
}

/// Changes the Providers of the installation in `home` as `asked` says:
/// `first NAME`, `remove NAME`, `limit NAME TOKENS` or `limit NAME none`.
/// Says what was done; the text of a failure is for the Owner.
pub async fn change_providers(home: &Path, asked: &[&str]) -> Result<String, String> {
    let mut config = Config::load(home).map_err(|error| error.to_string())?;
    let mut all = config.providers();
    let first_was = all.first().map(|first| first.name.clone());
    let mut removed = None;
    let place = |name: &str| {
        all.iter().position(|provider| provider.name == name.to_lowercase()).ok_or_else(|| {
            let names: Vec<&str> = all.iter().map(|provider| provider.name.as_str()).collect();
            format!("No provider called \"{name}\" is set up. Set up: {}.", names.join(", "))
        })
    };
    let done = match asked {
        ["first", name] => {
            let at = place(name)?;
            give_each_its_key(&mut config, &Secrets::kept_in(home))?;
            let mut all = config.providers();
            let preferred = all.remove(at);
            all.insert(0, preferred);
            config.set_providers(all);
            format!("{} is now asked first.", name.to_lowercase())
        }
        ["remove", name] => {
            let at = place(name)?;
            if all.len() == 1 {
                return Err("This is the only provider: set another up before removing it.".into());
            }
            give_each_its_key(&mut config, &Secrets::kept_in(home))?;
            let mut all = config.providers();
            let gone = all.remove(at);
            config.set_providers(all);
            let done = format!("{} is removed.", gone.name);
            removed = Some(gone);
            done
        }
        // One more, asked after the others: its key was given beforehand.
        ["add", name, model] => {
            let name = name.to_lowercase();
            let Some(known) = crate::known_providers().iter().find(|known| known.name == name)
            else {
                let names: Vec<&str> =
                    crate::known_providers().iter().map(|known| known.name).collect();
                return Err(format!(
                    "No provider is known as \"{name}\". Known: {}.",
                    names.join(", ")
                ));
            };
            if name == "chatgpt" {
                return Err("A ChatGPT plan is connected by signing in to it, not added.".into());
            }
            if all.iter().any(|provider| provider.name == name) {
                return Err(format!("{name} is already set up."));
            }
            let secrets = Secrets::kept_in(home);
            let key = key_of(&name);
            if known.needs_key && secrets.get(&key).map_err(|error| error.to_string())?.is_none() {
                return Err(format!("{name} needs its API key first."));
            }
            give_each_its_key(&mut config, &secrets)?;
            let mut all = config.providers();
            all.push(ProviderChoice {
                name: name.clone(),
                model: (*model).to_owned(),
                key: known.needs_key.then_some(key),
                ..ProviderChoice::default()
            });
            config.set_providers(all);
            format!(
                "{name} is added, with the model {model}: it is asked when those before it cannot answer."
            )
        }
        ["limit", name, tokens] => {
            let at = place(name)?;
            let limit = match *tokens {
                "none" | "off" | "0" => None,
                tokens => {
                    Some(tokens.replace([',', '_', ' '], "").parse::<u64>().map_err(|_| {
                        "A limit is a number of tokens a day, or \"none\".".to_owned()
                    })?)
                }
            };
            all[at].daily_tokens = limit;
            let name = all[at].name.clone();
            config.set_providers(all);
            match limit {
                Some(limit) => format!("{name} may use {} tokens a day.", grouped(limit)),
                None => format!("{name} has no limit."),
            }
        }
        _ => {
            return Err(
                "Usage: providers add NAME MODEL | providers first NAME | providers remove \
                        NAME | providers limit NAME TOKENS (or none)"
                    .into(),
            );
        }
    };
    // The lighter model was one of the Provider that came first: it is not
    // asked of another.
    if config.provider.as_ref().map(|first| &first.name) != first_was.as_ref() {
        config.light_model = None;
    }
    config.save(home).map_err(|error| error.to_string())?;
    // Only once it is no longer listed is what let it in let go of.
    if let Some(removed) = removed {
        if let Some(key) = &removed.key {
            let _ = Secrets::kept_in(home).remove(key);
        }
        // A plan signed in to is signed out of, at its Provider too.
        if removed.name == "chatgpt" {
            let chatgpt = crate::ChatGpt::kept_in(home, &removed.model);
            let chatgpt = match &removed.address {
                Some(address) => chatgpt.at(address),
                None => chatgpt,
            };
            chatgpt.sign_out().await;
        }
    }
    Ok(done)
}
