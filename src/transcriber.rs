//! Writing down what was said in a recording: by a Provider that offers it
//! to whoever has an API Key, or on the machine itself, by whisper.cpp, by
//! a local server, or by any program the Owner chooses.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;

/// The largest recording sent to a Provider: what these Providers take.
const LIMIT: u64 = 25_000_000;
/// How long a program of the machine may take over one recording.
const PATIENCE: Duration = Duration::from_secs(900);

/// Something that writes speech down.
pub struct Transcriber {
    engine: Engine,
}

enum Engine {
    /// OpenAI's transcription endpoint, at a Provider's or on the machine.
    Endpoint { http: reqwest::Client, endpoint: String, api_key: Option<String>, model: String },
    /// whisper.cpp's own program, run once for each recording.
    WhisperCpp {
        program: PathBuf,
        model: PathBuf,
        /// What turns any recording into the plain one whisper.cpp reads.
        converter: Option<PathBuf>,
        language: String,
    },
    /// A command of the Owner's, `{file}` standing for the recording; what
    /// it prints is what was said.
    Command(String),
}

/// The media type of `file` when it is a recording, by its extension.
pub(crate) fn sound_type(file: &Path) -> Option<&'static str> {
    match file.extension()?.to_str()?.to_lowercase().as_str() {
        "ogg" | "oga" | "opus" => Some("audio/ogg"),
        "mp3" | "mpga" => Some("audio/mpeg"),
        "m4a" => Some("audio/mp4"),
        "wav" => Some("audio/wav"),
        "flac" => Some("audio/flac"),
        _ => None,
    }
}

/// `text` as one word of a shell command, whatever it holds.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Runs `command` to its end; what it printed, or why it failed.
async fn run(mut command: tokio::process::Command, named: &str) -> Result<String, String> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let running = command.spawn().map_err(|error| format!("{named} could not be run: {error}"))?;
    let done = tokio::time::timeout(PATIENCE, running.wait_with_output())
        .await
        .map_err(|_| format!("{named} took more than {} minutes", PATIENCE.as_secs() / 60))?
        .map_err(|error| format!("{named} could not be run: {error}"))?;
    if !done.status.success() {
        let said = String::from_utf8_lossy(&done.stderr);
        let said: String =
            said.trim().chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!(
            "{named} failed: {}",
            if said.is_empty() { "no reason given" } else { &said }
        ));
    }
    Ok(String::from_utf8_lossy(&done.stdout).trim().to_owned())
}

impl Transcriber {
    /// A Provider's speech-to-text. `base_url` is its API root, such as
    /// `https://api.openai.com/v1`.
    pub fn new(
        base_url: impl AsRef<str>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self::endpoint(base_url.as_ref(), Some(api_key.into()), model.into())
    }

    /// A server on the Owner's own machine or network that speaks OpenAI's
    /// transcription protocol, and takes no key.
    pub fn local_server(base_url: impl AsRef<str>, model: impl Into<String>) -> Self {
        Self::endpoint(base_url.as_ref(), None, model.into())
    }

    fn endpoint(base_url: &str, api_key: Option<String>, model: String) -> Self {
        let endpoint = format!("{}/audio/transcriptions", base_url.trim_end_matches('/'));
        Self { engine: Engine::Endpoint { http: reqwest::Client::new(), endpoint, api_key, model } }
    }

    /// whisper.cpp on this machine: its `program` (whisper-cli) and the
    /// file of the model it listens with.
    pub fn whisper_cpp(program: impl Into<PathBuf>, model: impl Into<PathBuf>) -> Self {
        Self {
            engine: Engine::WhisperCpp {
                program: program.into(),
                model: model.into(),
                converter: None,
                language: "auto".into(),
            },
        }
    }

    /// Has recordings turned into the plain ones whisper.cpp reads by this
    /// program (ffmpeg).
    pub fn converting_with(mut self, ffmpeg: impl Into<PathBuf>) -> Self {
        if let Engine::WhisperCpp { converter, .. } = &mut self.engine {
            *converter = Some(ffmpeg.into());
        }
        self
    }

    /// Says which language is spoken, rather than leaving it to be found.
    pub fn in_language(mut self, spoken: impl Into<String>) -> Self {
        if let Engine::WhisperCpp { language, .. } = &mut self.engine {
            *language = spoken.into();
        }
        self
    }

    /// Any program: `command` is run by the shell with `{file}` replaced by
    /// the recording, and what it prints is what was said.
    pub fn command(command: impl Into<String>) -> Self {
        Self { engine: Engine::Command(command.into()) }
    }

    /// What is said in `file`. The text of a failure is for the model to
    /// pass on to the Owner.
    pub async fn written(&self, file: &Path) -> Result<String, String> {
        let said = match &self.engine {
            Engine::Endpoint { http, endpoint, api_key, model } => {
                Self::asked_of(http, endpoint, api_key.as_deref(), model, file).await?
            }
            Engine::WhisperCpp { program, model, converter, language } => {
                Self::heard_by_whisper(program, model, converter.as_deref(), language, file).await?
            }
            Engine::Command(command) => {
                let filled = command.replace("{file}", &quoted(&file.to_string_lossy()));
                let mut shell = tokio::process::Command::new("sh");
                shell.arg("-c").arg(filled);
                run(shell, "the transcription command").await?
            }
        };
        match said.trim() {
            "" => Err("nothing could be made out in it".into()),
            said => Ok(said.to_owned()),
        }
    }

    async fn asked_of(
        http: &reqwest::Client,
        endpoint: &str,
        api_key: Option<&str>,
        model: &str,
        file: &Path,
    ) -> Result<String, String> {
        let kind = sound_type(file).ok_or("it is not a recording of a kind I can send")?;
        let size = std::fs::metadata(file).map_err(|error| error.to_string())?.len();
        if size > LIMIT {
            return Err(format!("it is too long to be written down ({} MB)", size / 1_000_000));
        }
        let bytes = std::fs::read(file).map_err(|error| error.to_string())?;
        // Under a name whose ending is known: Telegram's voice messages are
        // Ogg, which some programs name .oga.
        let mut name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if kind == "audio/ogg" && !name.to_lowercase().ends_with(".ogg") {
            name = Path::new(&name).with_extension("ogg").to_string_lossy().into_owned();
        }
        let recording = reqwest::multipart::Part::bytes(bytes)
            .file_name(name)
            .mime_str(kind)
            .map_err(|error| error.to_string())?;
        let form =
            reqwest::multipart::Form::new().text("model", model.to_owned()).part("file", recording);
        let asking = http.post(endpoint).multipart(form).timeout(Duration::from_secs(300));
        let asking = match api_key {
            Some(key) => asking.bearer_auth(key),
            None => asking,
        };
        let answer = asking.send().await.map_err(|error| {
            format!("what writes speech down could not be reached ({})", error.without_url())
        })?;
        let status = answer.status();
        let said = answer.json::<Value>().await.unwrap_or_default();
        if !status.is_success() {
            let why = said["error"]["message"]
                .as_str()
                .or(said["error"].as_str())
                .or(said["detail"].as_str())
                .unwrap_or("no reason given");
            return Err(format!("HTTP {}: {why}", status.as_u16()));
        }
        Ok(said["text"].as_str().unwrap_or_default().to_owned())
    }

    async fn heard_by_whisper(
        program: &Path,
        model: &Path,
        converter: Option<&Path>,
        language: &str,
        file: &Path,
    ) -> Result<String, String> {
        // whisper.cpp is sure to read 16-bit WAV only: anything else is
        // turned into that first, where the machine has what does it.
        let plain = file.extension().is_some_and(|ending| ending.eq_ignore_ascii_case("wav"));
        let converted = match converter.filter(|_| !plain) {
            None => None,
            Some(ffmpeg) => {
                let into = file.with_extension(format!("{}.wav", std::process::id()));
                let mut converting = tokio::process::Command::new(ffmpeg);
                converting
                    .args(["-nostdin", "-loglevel", "error", "-y", "-i"])
                    .arg(file)
                    .args(["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
                    .arg(&into);
                if let Err(why) = run(converting, "ffmpeg").await {
                    let _ = std::fs::remove_file(&into);
                    return Err(why);
                }
                Some(into)
            }
        };
        let mut hearing = tokio::process::Command::new(program);
        hearing
            .arg("-m")
            .arg(model)
            .args(["-l", language, "-nt", "-np", "-f"])
            .arg(converted.as_deref().unwrap_or(file));
        let heard = run(hearing, &program.to_string_lossy()).await;
        if let Some(converted) = &converted {
            let _ = std::fs::remove_file(converted);
        }
        heard.map_err(|why| {
            if converter.is_none() && !plain {
                format!(
                    "{why}. whisper.cpp reads plain WAV recordings; with ffmpeg on the machine \
                     (apt install ffmpeg) Orbite converts the others first"
                )
            } else {
                why
            }
        })
    }
}
