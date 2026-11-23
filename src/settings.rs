//! The settings an Owner changes, by name, wherever they are kept: the same
//! from the Conversation and from the `orbite` command.

use std::path::Path;

use chrono::NaiveTime;

use crate::{Config, Store};

/// The settings, in the order they are shown.
pub const SETTINGS: [&str; 12] = [
    "permission",
    "quiet",
    "wakeups",
    "secrets",
    "updates",
    "language",
    "name",
    "shape",
    "colour",
    "model",
    "light",
    "voice",
];

/// The settings that are read when Orbite starts: changing one takes a restart.
pub const TAKES_A_RESTART: [&str; 7] =
    ["language", "name", "shape", "colour", "model", "light", "voice"];

/// The settings of one installation.
pub struct Settings<'a> {
    store: &'a Store,
    /// The installation's folder, when it has one.
    home: Option<&'a Path>,
    /// Minutes between Wake-ups when the Owner has not said.
    usual_wake_ups: u32,
}

impl<'a> Settings<'a> {
    pub fn of(store: &'a Store, home: Option<&'a Path>) -> Self {
        Self { store, home, usual_wake_ups: 0 }
    }

    /// How often the Agent wakes up when the Owner has not said, in minutes.
    pub fn waking_up_by_default_every(mut self, minutes: u32) -> Self {
        self.usual_wake_ups = minutes;
        self
    }

    fn config(&self) -> Config {
        self.home.and_then(|home| Config::load(home).ok()).unwrap_or_default()
    }

    fn kept(&self, name: &str) -> Result<Option<String>, String> {
        self.store.setting(name).map_err(|error| error.to_string())
    }

    /// The value of the setting called `key`, as it is written to set it.
    pub fn get(&self, key: &str) -> Result<String, String> {
        Ok(match key {
            "permission" => self.kept("permission_level")?.unwrap_or_else(|| "careful".into()),
            "quiet" => self.kept("quiet_hours")?.unwrap_or_else(|| "off".into()),
            "wakeups" => {
                let minutes =
                    self.kept("wake_up_minutes")?.and_then(|minutes| minutes.parse::<u32>().ok());
                match minutes.unwrap_or(self.usual_wake_ups) {
                    0 => "off".into(),
                    minutes => minutes.to_string(),
                }
            }
            "secrets" if self.kept("secrets_strict")?.is_some() => "strict".into(),
            "secrets" => on_or_off(self.kept("secrets_in_conversation")?.is_some()).into(),
            "updates" => on_or_off(self.config().automatic_updates != Some(false)).into(),
            "language" => self.config().language.unwrap_or_else(|| "en".into()),
            "model" => self.config().provider.map_or_else(|| "none".into(), |chosen| chosen.model),
            "name" => self.config().agent_name.unwrap_or_else(|| "Orbite".into()),
            // What the Agent looks like.
            "shape" => {
                let config = self.config();
                crate::Look::called(config.agent_shape.as_deref(), None).shape.into()
            }
            "colour" => {
                let config = self.config();
                crate::Look::called(None, config.agent_colour.as_deref()).colour.into()
            }
            // The model that does the light work: summaries and Wake-ups.
            "light" => self.config().light_model.unwrap_or_else(|| "off".into()),
            // What writes down what the Owner says aloud.
            "voice" => match self.config().transcription {
                None => "provider".into(),
                Some(chosen) => match (chosen.engine.as_str(), &chosen.model) {
                    ("whisper.cpp", Some(model)) => format!("whisper.cpp ({model})"),
                    (engine, _) => engine.to_owned(),
                },
            },
            other => return Err(unknown(other)),
        })
    }

    /// The values the setting called `key` is usually given.
    pub fn choices(key: &str) -> &'static [&'static str] {
        match key {
            "permission" => &["careful", "balanced", "free"],
            "quiet" => &["off", "22:00-07:00", "23:00-08:00", "00:00-09:00"],
            "wakeups" => &["off", "60", "240", "720"],
            // strict: what speaks of a password is held back from the AI
            // provider until the Owner says what it is.
            "secrets" => &["off", "strict", "on"],
            "updates" => &["off", "on"],
            "language" => &["en", "fr"],
            "shape" => &crate::SHAPES,
            "colour" => &[
                "white", "clay", "red", "orange", "amber", "green", "teal", "blue", "violet",
                "pink", "grey",
            ],
            "light" => &["off"],
            "voice" => &["provider", "whisper.cpp", "server", "command", "off"],
            _ => &[],
        }
    }

    /// Gives the setting called `key` the value `value`; the error says what
    /// it can be given.
    pub fn set(&self, key: &str, written: &str) -> Result<(), String> {
        let value = written.trim().to_lowercase();
        let keep = |name: &str, value: Option<&str>| {
            self.store.set_setting(name, value).map_err(|error| error.to_string())
        };
        let one_of = |choices: &[&str]| {
            if choices.contains(&value.as_str()) {
                Ok(())
            } else {
                Err(format!("{key} is one of: {}.", choices.join(", ")))
            }
        };
        match key {
            "permission" => {
                one_of(Self::choices(key))?;
                keep("permission_level", Some(&value))
            }
            "quiet" if value == "off" => keep("quiet_hours", None),
            "quiet" => {
                let read = |time: &str| NaiveTime::parse_from_str(time.trim(), "%H:%M").ok();
                let times = value
                    .split_once('-')
                    .and_then(|(from, until)| Some((read(from)?, read(until)?)));
                let Some((from, until)) = times else {
                    return Err(
                        "quiet is off, or two times as HH:MM-HH:MM, such as 22:00-07:00.".into()
                    );
                };
                keep(
                    "quiet_hours",
                    Some(&format!("{}-{}", from.format("%H:%M"), until.format("%H:%M"))),
                )
            }
            "wakeups" => {
                let minutes = if value == "off" { Some(0) } else { value.parse::<u32>().ok() };
                match minutes.filter(|minutes| *minutes <= 7 * 24 * 60) {
                    Some(minutes) => keep("wake_up_minutes", Some(&minutes.to_string())),
                    None => Err("wakeups is off, or a number of minutes, such as 240.".into()),
                }
            }
            "secrets" => {
                one_of(Self::choices(key))?;
                keep("secrets_strict", (value == "strict").then_some("on"))?;
                keep("secrets_in_conversation", (value == "on").then_some("on"))
            }
            "updates" | "language" => {
                one_of(Self::choices(key))?;
                let Some(home) = self.home else {
                    return Err(format!(
                        "{key} is kept in Orbite's folder, and there is none here."
                    ));
                };
                let mut config = Config::load(home).map_err(|error| error.to_string())?;
                if key == "updates" {
                    config.automatic_updates = (value == "off").then_some(false);
                } else {
                    config.language = Some(value);
                }
                config.save(home).map_err(|error| error.to_string())
            }
            "voice" => {
                one_of(Self::choices(key))?;
                let Some(home) = self.home else {
                    return Err("voice is kept in Orbite's folder, and there is none here.".into());
                };
                let mut config = Config::load(home).map_err(|error| error.to_string())?;
                let mut chosen = config.transcription.clone().unwrap_or_default();
                // A way of the machine's own is set up there, with its model
                // or address; from here it is only gone back to.
                let set_up = match value.as_str() {
                    "whisper.cpp" => {
                        chosen.model.is_some()
                            && chosen.address.is_none()
                            && chosen.command.is_none()
                    }
                    "server" => chosen.address.is_some(),
                    "command" => chosen.command.is_some(),
                    _ => true,
                };
                if !set_up {
                    return Err(format!(
                        "{value} is not set up yet. Set it up on the machine first: orbite transcription"
                    ));
                }
                chosen.engine = value;
                config.transcription = Some(chosen);
                config.save(home).map_err(|error| error.to_string())
            }
            "model" => {
                // As written: the names of models tell capitals apart.
                let model = written.trim();
                let Some(home) = self.home else {
                    return Err("model is kept in Orbite's folder, and there is none here.".into());
                };
                let mut config = Config::load(home).map_err(|error| error.to_string())?;
                let Some(chosen) = config.provider.as_mut() else {
                    return Err(
                        "No Provider is set up yet: run `orbite setup` on the machine.".into()
                    );
                };
                if model.is_empty() || model.contains(char::is_whitespace) {
                    return Err(
                        "model is the name of a model of your Provider, such as the one shown."
                            .into(),
                    );
                }
                chosen.model = model.to_owned();
                config.save(home).map_err(|error| error.to_string())
            }
            "shape" | "colour" => {
                let Some(home) = self.home else {
                    return Err(format!(
                        "{key} is kept in Orbite's folder, and there is none here."
                    ));
                };
                let asked = written.trim().to_lowercase();
                let known: Vec<&str> = match key {
                    "shape" => crate::SHAPES.to_vec(),
                    _ => crate::COLOURS.iter().map(|(colour, ..)| *colour).collect(),
                };
                if !known.contains(&asked.as_str()) {
                    return Err(format!("{key} is one of: {}.", known.join(", ")));
                }
                let mut config = Config::load(home).map_err(|error| error.to_string())?;
                match key {
                    "shape" => config.agent_shape = Some(asked),
                    _ => config.agent_colour = Some(asked),
                }
                config.save(home).map_err(|error| error.to_string())
            }
            "name" => {
                // As written: a name has its capitals.
                let name = written.trim();
                let Some(home) = self.home else {
                    return Err("name is kept in Orbite's folder, and there is none here.".into());
                };
                if name.is_empty() || name.chars().count() > 40 || name.contains('\n') {
                    return Err(
                        "name is what your agent is called: a few words on one line.".into()
                    );
                }
                let mut config = Config::load(home).map_err(|error| error.to_string())?;
                config.agent_name = Some(name.to_owned());
                config.save(home).map_err(|error| error.to_string())
            }
            "light" => {
                let model = written.trim();
                let Some(home) = self.home else {
                    return Err("light is kept in Orbite's folder, and there is none here.".into());
                };
                if model.is_empty() || model.contains(char::is_whitespace) {
                    return Err("light is the name of a model of your Provider, or off.".into());
                }
                let mut config = Config::load(home).map_err(|error| error.to_string())?;
                config.light_model =
                    if model.eq_ignore_ascii_case("off") { None } else { Some(model.to_owned()) };
                config.save(home).map_err(|error| error.to_string())
            }
            other => Err(unknown(other)),
        }
    }
}

fn on_or_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

fn unknown(key: &str) -> String {
    format!("There is no setting called {key}. The settings are: {}.", SETTINGS.join(", "))
}
