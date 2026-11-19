//! How an installation has speech written down: by a Provider, on the
//! machine itself, or not at all; and what its Owner is told before
//! choosing a model the machine would carry badly.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::{Config, Language, Transcriber, Transcription};

/// A model of whisper.cpp, as its project publishes it.
pub struct Model {
    pub name: &'static str,
    /// The size of its file, in bytes.
    pub size: u64,
    /// The SHA-256 of its file.
    sum: &'static str,
    /// The memory it takes to run, in MB, where the project says.
    memory: Option<u64>,
}

/// The models offered, the lightest first. Sizes and sums are those of
/// huggingface.co/ggerganov/whisper.cpp, read on 2026-11-19; the memory is
/// the project's own table.
pub const MODELS: &[Model] = &[
    Model {
        name: "tiny",
        size: 77_691_713,
        sum: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        memory: Some(273),
    },
    Model {
        name: "base-q5_1",
        size: 59_707_625,
        sum: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        memory: None,
    },
    Model {
        name: "base",
        size: 147_951_465,
        sum: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        memory: Some(388),
    },
    Model {
        name: "small-q5_1",
        size: 190_085_487,
        sum: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        memory: None,
    },
    Model {
        name: "small",
        size: 487_601_967,
        sum: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        memory: Some(852),
    },
    Model {
        name: "medium-q5_0",
        size: 539_212_467,
        sum: "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
        memory: None,
    },
    Model {
        name: "large-v3-turbo-q5_0",
        size: 574_041_195,
        sum: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        memory: None,
    },
    Model {
        name: "medium",
        size: 1_533_763_059,
        sum: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        memory: Some(2100),
    },
    Model {
        name: "large-v3-turbo",
        size: 1_624_555_275,
        sum: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        memory: None,
    },
    Model {
        name: "large-v3-q5_0",
        size: 1_081_140_203,
        sum: "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        memory: None,
    },
    Model {
        name: "large-v3",
        size: 3_095_033_483,
        sum: "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        memory: Some(3900),
    },
];

const MODELS_AT: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// What a machine has to run a model with.
#[derive(Debug, Clone, Copy)]
pub struct Machine {
    pub memory_mb: u64,
    pub cores: u64,
}

impl Machine {
    /// This machine. ORBITE_MACHINE_MEMORY_MB and ORBITE_MACHINE_CORES say
    /// otherwise, for trying out what a smaller one would be told.
    pub fn this() -> Self {
        let said = |name: &str| std::env::var(name).ok().and_then(|value| value.parse().ok());
        let memory = std::fs::read_to_string("/proc/meminfo").ok().and_then(|about| {
            let line = about.lines().find(|line| line.starts_with("MemTotal:"))?;
            line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb / 1024)
        });
        let cores = std::thread::available_parallelism().map_or(1, |cores| cores.get() as u64);
        Self {
            memory_mb: said("ORBITE_MACHINE_MEMORY_MB").or(memory).unwrap_or(0),
            cores: said("ORBITE_MACHINE_CORES").unwrap_or(cores),
        }
    }
}

/// What an Owner should know before choosing a model.
#[derive(Debug, Default, PartialEq)]
pub struct Advice {
    /// What may disappoint, each in a sentence.
    pub warnings: Vec<String>,
    /// True when the model cannot run on the machine at all.
    pub does_not_fit: bool,
}

impl Model {
    pub fn named(name: &str) -> Option<&'static Model> {
        MODELS.iter().find(|model| model.name == name)
    }

    /// The memory it takes, in MB: the project's figure, or at least the
    /// size of its file where the project gives none.
    fn needs(&self) -> (u64, bool) {
        match self.memory {
            Some(memory) => (memory, true),
            None => (self.size / 1_000_000, false),
        }
    }

    /// What `machine` would make of this model.
    pub fn advice(&self, machine: Machine, language: Language) -> Advice {
        let say = |english: String, french: String| match language {
            Language::French => french,
            Language::English => english,
        };
        let mut advice = Advice::default();
        if self.name.starts_with("tiny") || self.name.starts_with("base") {
            advice.warnings.push(say(
                "a very small model: quick, but it makes many mistakes, more so in languages other \
                 than English. `small` is the smallest that listens well."
                    .into(),
                "un très petit modèle : rapide, mais il fait beaucoup d'erreurs, surtout hors de \
                 l'anglais. `small` est le plus petit qui écoute bien."
                    .into(),
            ));
        }
        if self.name.contains("-q") {
            advice.warnings.push(say(
                "a compressed model: lighter and a little less exact than the one it comes from."
                    .into(),
                "un modèle compressé : plus léger et un peu moins exact que celui dont il vient."
                    .into(),
            ));
        }
        let (needs, stated) = self.needs();
        let at_least =
            if stated { String::new() } else { say("at least ".into(), "au moins ".into()) };
        if machine.memory_mb > 0 && needs >= machine.memory_mb {
            advice.does_not_fit = true;
            advice.warnings.push(say(
                format!("it takes {at_least}{needs} MB of memory and this machine has {} MB: it will not run.", machine.memory_mb),
                format!("il lui faut {at_least}{needs} Mo de mémoire et cette machine en a {} : il ne tournera pas.", machine.memory_mb),
            ));
        } else if machine.memory_mb > 0 && needs * 2 > machine.memory_mb {
            advice.warnings.push(say(
                format!("it takes {at_least}{needs} MB of this machine's {} MB of memory: everything else, the agent's browser first, will be short of it while a message is written down.", machine.memory_mb),
                format!("il lui faut {at_least}{needs} Mo sur les {} Mo de mémoire de cette machine : tout le reste, à commencer par le navigateur de l'agent, en manquera pendant qu'un message est transcrit.", machine.memory_mb),
            ));
        }
        if machine.cores <= 2 && self.size > 400_000_000 {
            advice.warnings.push(say(
                format!("with {} processor core(s), a model this large will take longer to write a message down than the message lasts.", machine.cores),
                format!("avec {} cœur(s) de processeur, un modèle de cette taille mettra plus de temps à transcrire un message que le message ne dure.", machine.cores),
            ));
        }
        advice
    }
}

/// Where `program` is on this machine, looked for as a shell would.
pub fn found(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        return Path::new(program).is_file().then(|| PathBuf::from(program));
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path).map(|folder| folder.join(program)).find(|file| file.is_file())
    })
}

/// What writes speech down for the installation in `home`, as its Owner
/// chose; `None` when a Provider does, or nothing.
pub fn chosen(home: &Path, chosen: &Transcription) -> Result<Option<Transcriber>, String> {
    Ok(Some(match chosen.engine.as_str() {
        "whisper.cpp" => {
            let program = chosen.program.clone().unwrap_or_else(|| "whisper-cli".into());
            let program = found(&program).ok_or(format!(
                "whisper.cpp is chosen to write speech down, but {program} is not on this machine."
            ))?;
            let model = model_file(home, chosen.model.as_deref().unwrap_or("small"));
            let mut transcriber = Transcriber::whisper_cpp(program, model);
            if let Some(ffmpeg) = found("ffmpeg") {
                transcriber = transcriber.converting_with(ffmpeg);
            }
            match chosen.language.as_deref().filter(|language| !language.is_empty()) {
                Some(language) => transcriber.in_language(language),
                None => transcriber,
            }
        }
        "server" => Transcriber::local_server(
            chosen.address.as_deref().ok_or("The transcription server has no address.")?,
            chosen.model.clone().unwrap_or_default(),
        ),
        "command" => Transcriber::command(
            chosen.command.clone().ok_or("The transcription command is missing.")?,
        ),
        _ => return Ok(None),
    }))
}

/// The file of the model called `name`, or `name` itself when it is a file
/// of the Owner's own.
fn model_file(home: &Path, name: &str) -> PathBuf {
    if name.contains('/') {
        return PathBuf::from(name);
    }
    home.join("models").join(format!("ggml-{name}.bin"))
}

/// Ten seconds of silence, as the plain recording whisper.cpp reads.
fn silence() -> Vec<u8> {
    let samples: u32 = 16_000 * 10;
    let mut wav = Vec::with_capacity(44 + samples as usize * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&32_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples * 2).to_le_bytes());
    wav.resize(44 + samples as usize * 2, 0);
    wav
}

/// Fetches the model called `model` into `home`, whole and as published, or not at all.
async fn fetch(home: &Path, model: &Model, shown: &mut impl Write) -> Result<PathBuf, String> {
    let file = model_file(home, model.name);
    if file.is_file() {
        return Ok(file);
    }
    let from = std::env::var("ORBITE_MODELS_URL").unwrap_or_else(|_| MODELS_AT.into());
    let address = format!("{}/ggml-{}.bin", from.trim_end_matches('/'), model.name);
    let _ = writeln!(
        shown,
        "Downloading {} ({} MB) from {address} ...",
        model.name,
        model.size / 1_000_000
    );
    let wrong =
        |error: &dyn std::fmt::Display| format!("The model could not be downloaded: {error}");
    std::fs::create_dir_all(file.parent().unwrap_or(home)).map_err(|error| wrong(&error))?;
    let arriving = file.with_extension("downloading");
    let mut response = reqwest::Client::new()
        .get(&address)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| wrong(&error.without_url()))?;
    let mut written = std::fs::File::create(&arriving).map_err(|error| wrong(&error))?;
    let mut sum = Sha256::new();
    loop {
        match response.chunk().await {
            Ok(Some(piece)) => {
                sum.update(&piece);
                written.write_all(&piece).map_err(|error| wrong(&error))?;
            }
            Ok(None) => break,
            Err(error) => {
                let _ = std::fs::remove_file(&arriving);
                return Err(wrong(&error.without_url()));
            }
        }
    }
    let sum: String = sum.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    if sum != model.sum {
        let _ = std::fs::remove_file(&arriving);
        return Err(
            "What was downloaded does not match the model its project published: it was not kept."
                .into(),
        );
    }
    std::fs::rename(&arriving, &file).map_err(|error| wrong(&error))?;
    Ok(file)
}

/// Shows or changes how the installation in `home` has speech written
/// down, as `asked` says; what is to be shown goes to `shown`.
pub async fn transcription(
    home: &Path,
    asked: &[&str],
    shown: &mut impl Write,
) -> Result<(), String> {
    let mut config = Config::load(home).map_err(|error| error.to_string())?;
    let language = Language::of(config.language.as_deref());
    let machine = Machine::this();
    let anyway = asked.contains(&"--anyway");
    let asked: Vec<&str> = asked.iter().copied().filter(|word| *word != "--anyway").collect();
    let out = |shown: &mut dyn Write, line: String| {
        let _ = writeln!(shown, "{line}");
    };
    let mut choice = config.transcription.clone().unwrap_or_default();
    match asked[..] {
        [] => {
            let current = match choice.engine.as_str() {
                "" | "provider" => {
                    "provider (the first of your providers that offers it for an API key)"
                        .to_owned()
                }
                "off" => "off".to_owned(),
                "whisper.cpp" => {
                    format!("whisper.cpp, model {}", choice.model.as_deref().unwrap_or("small"))
                }
                "server" => format!("server at {}", choice.address.as_deref().unwrap_or("?")),
                other => other.to_owned(),
            };
            out(shown, format!("Speech is written down by: {current}"));
            if let Some(spoken) = &choice.language {
                out(shown, format!("Language: {spoken}"));
            }
            out(
                shown,
                format!(
                    "\nThis machine: {} MB of memory, {} processor core(s).",
                    machine.memory_mb, machine.cores
                ),
            );
            out(
                shown,
                format!(
                    "whisper-cli: {}    ffmpeg: {}",
                    found("whisper-cli").map_or("not found".into(), |at| at.display().to_string()),
                    found("ffmpeg").map_or("not found".into(), |at| at.display().to_string())
                ),
            );
            out(shown, "\nModels of whisper.cpp (orbite transcription whisper.cpp NAME):".into());
            for model in MODELS {
                let advice = model.advice(machine, language);
                let mark = if advice.does_not_fit {
                    "⛔"
                } else if advice.warnings.is_empty() {
                    "✅"
                } else {
                    "⚠️"
                };
                out(shown, format!("  {mark} {:<20} {:>5} MB", model.name, model.size / 1_000_000));
                for warning in advice.warnings {
                    out(shown, format!("       {warning}"));
                }
            }
            out(shown, "\nOther ways: orbite transcription provider | off | server ADDRESS MODEL | command 'PROGRAM ... {file}' | language CODE (or auto)".into());
            return Ok(());
        }
        ["provider"] | ["off"] => {
            // What was set up for the machine is kept, to go back to it.
            choice.engine = asked[0].into();
        }
        ["language", spoken] => {
            choice.language = (spoken != "auto").then(|| spoken.to_owned());
        }
        ["server", address, model] => {
            choice = Transcription {
                engine: "server".into(),
                address: Some(address.into()),
                model: Some(model.into()),
                language: choice.language,
                ..Default::default()
            };
        }
        ["command", command] => {
            if !command.contains("{file}") {
                return Err(
                    "The command must say where the recording goes: write {file} in it.".into()
                );
            }
            choice = Transcription {
                engine: "command".into(),
                command: Some(command.into()),
                ..Default::default()
            };
        }
        ["whisper.cpp"] | ["whisper.cpp", _] => {
            let name = asked.get(1).copied().unwrap_or("small");
            let program = found("whisper-cli").ok_or(
                "whisper.cpp is not on this machine (whisper-cli was not found). On Ubuntu 26.04: \
                 sudo apt install whisper.cpp ffmpeg. Elsewhere, build it from \
                 https://github.com/ggml-org/whisper.cpp and put whisper-cli on the PATH. Nothing \
                 was changed.",
            )?;
            let file = if name.contains('/') {
                if !Path::new(name).is_file() {
                    return Err(format!("There is no model file at {name}. Nothing was changed."));
                }
                PathBuf::from(name)
            } else {
                let model = Model::named(name).ok_or_else(|| {
                    let names: Vec<&str> = MODELS.iter().map(|model| model.name).collect();
                    format!("\"{name}\" is not a model I know. Models: {}; or the path of a model file of your own.", names.join(", "))
                })?;
                let advice = model.advice(machine, language);
                for warning in &advice.warnings {
                    out(shown, format!("⚠️ {warning}"));
                }
                if advice.does_not_fit && !anyway {
                    return Err(
                        "Nothing was changed. To choose it all the same, add --anyway.".into()
                    );
                }
                fetch(home, model, shown).await?
            };
            if found("ffmpeg").is_none() {
                out(shown, "⚠️ ffmpeg is not on this machine: whisper.cpp reads plain WAV recordings, and Telegram's voice messages are not. Install it (sudo apt install ffmpeg) for them to be written down.".into());
            }
            // How this machine really carries it: ten seconds of silence, timed.
            let sample = home.join("models").join("ten-seconds.wav");
            let _ = std::fs::create_dir_all(home.join("models"));
            std::fs::write(&sample, silence()).map_err(|error| error.to_string())?;
            let started = Instant::now();
            let tried = Transcriber::whisper_cpp(&program, &file).written(&sample).await;
            let took = started.elapsed().as_secs_f32();
            let _ = std::fs::remove_file(&sample);
            match tried {
                // Silence holds no words: that much is expected.
                Err(why) if !why.contains("nothing could be made out") => {
                    return Err(format!(
                        "whisper.cpp did not run with this model: {why}. Nothing was changed."
                    ));
                }
                _ => {}
            }
            out(shown, format!("This machine took {took:.1} s to listen to 10 seconds of sound."));
            if took > 10.0 {
                out(shown, "⚠️ That is slower than speech itself: a voice message will take longer to be written down than it lasts. A smaller model would be quicker.".into());
            }
            choice = Transcription {
                engine: "whisper.cpp".into(),
                model: Some(name.to_owned()),
                language: choice.language,
                ..Default::default()
            };
        }
        _ => {
            return Err("Usage: orbite transcription | provider | off | whisper.cpp [MODEL or FILE] [--anyway] | server ADDRESS MODEL | command 'PROGRAM ... {file}' | language CODE".into());
        }
    }
    config.transcription = Some(choice);
    config.save(home).map_err(|error| error.to_string())?;
    out(shown, "Saved. It applies at the next start of Orbite (as a service: sudo systemctl restart orbite).".into());
    Ok(())
}
