//! Writing down what was said in a recording, by a Provider that offers it
//! to whoever has an API Key: OpenAI's transcription endpoint, which Groq
//! also serves.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

/// The largest recording sent to be written down: what these Providers take.
const LIMIT: u64 = 25_000_000;

/// A Provider's speech-to-text, reached with an API Key.
pub struct Transcriber {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
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

impl Transcriber {
    /// `base_url` is the Provider's API root, such as `https://api.openai.com/v1`.
    pub fn new(
        base_url: impl AsRef<str>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: format!("{}/audio/transcriptions", base_url.as_ref().trim_end_matches('/')),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    /// What is said in `file`. The text of a failure is for the model to
    /// pass on to the Owner.
    pub(crate) async fn written(&self, file: &Path) -> Result<String, String> {
        let kind = sound_type(file).ok_or("it is not a recording of a kind I can send")?;
        let size = std::fs::metadata(file).map_err(|error| error.to_string())?.len();
        if size > LIMIT {
            return Err(format!("it is too long to be written down ({} MB)", size / 1_000_000));
        }
        let bytes = std::fs::read(file).map_err(|error| error.to_string())?;
        // Under a name whose ending these Providers know: Telegram's voice
        // messages are Ogg, which some programs name .oga.
        let mut name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if kind == "audio/ogg" && !name.to_lowercase().ends_with(".ogg") {
            name = Path::new(&name).with_extension("ogg").to_string_lossy().into_owned();
        }
        let recording = reqwest::multipart::Part::bytes(bytes)
            .file_name(name)
            .mime_str(kind)
            .map_err(|error| error.to_string())?;
        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .part("file", recording);
        let answer = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|error| {
                format!("the provider that listens could not be reached ({})", error.without_url())
            })?;
        let status = answer.status();
        let said = answer.json::<Value>().await.unwrap_or_default();
        if !status.is_success() {
            let why = said["error"]["message"].as_str().unwrap_or("no reason given");
            return Err(format!("HTTP {}: {why}", status.as_u16()));
        }
        match said["text"].as_str().map(str::trim) {
            Some(text) if !text.is_empty() => Ok(text.to_owned()),
            _ => Err("nothing could be made out in it".into()),
        }
    }
}
