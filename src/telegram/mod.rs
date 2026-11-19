mod formatting;

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{Answer, Approval, Channel, Language, Step, attached};

/// How long Telegram may hold a request open waiting for a message.
const LONG_POLL: Duration = Duration::from_secs(50);

/// The longest text Telegram accepts in one message, in UTF-16 units.
const MESSAGE_LIMIT: usize = 4096;

/// The largest file Telegram lets a bot fetch.
const FETCH_LIMIT: u64 = 20_000_000;

/// How many steps of the work under way are shown at once.
const STEPS_SHOWN: usize = 5;

/// The Conversation in Telegram, through a bot the Owner created.
///
/// The bot asks Telegram for new messages (long polling), so the Host needs no
/// open port, domain or certificate. A task of its own does the asking, so
/// that nothing the Owner sends is lost while the Agent is busy elsewhere.
pub struct Telegram {
    bot: Arc<Bot>,
    /// What the Owner sent, once the listening task has started.
    listening: Option<(mpsc::UnboundedReceiver<Sent>, JoinHandle<()>)>,
    waiting: VecDeque<String>,
    /// What Telegram could not take when it was sent, oldest first.
    undelivered: VecDeque<String>,
    /// Taps on Approval buttons not yet matched to an Approval.
    taps: VecDeque<Tap>,
    /// The message showing the steps of the work under way, and those steps.
    progress: Option<(i64, Vec<String>)>,
    /// The Approval being asked: its message and the text shown.
    open_approval: Option<(i64, String)>,
    /// The message the Owner's latest words came in.
    last_received: Option<i64>,
}

/// How to reach Telegram as the bot.
struct Bot {
    http: reqwest::Client,
    api_root: String,
    token: String,
    owner: i64,
    retry_after: Duration,
    /// Where the files the Owner sends are saved, when there is somewhere to.
    inbox: Option<PathBuf>,
    /// The language of what the bot shows itself.
    language: Language,
}

/// Something the Owner sent to the bot.
enum Sent {
    /// What they wrote, and the message it came in.
    Text(String, i64),
    Tap(Tap),
}

/// The Owner's tap on a button under one of the bot's messages.
struct Tap {
    id: String,
    message: i64,
    choice: String,
}

impl Telegram {
    /// `token` is the bot's; `owner` is the Telegram account the Agent obeys.
    pub fn new(token: impl Into<String>, owner: i64) -> Self {
        Self {
            bot: Arc::new(Bot {
                http: reqwest::Client::new(),
                api_root: "https://api.telegram.org".into(),
                token: token.into(),
                owner,
                retry_after: Duration::from_secs(3),
                inbox: None,
                language: Language::English,
            }),
            listening: None,
            waiting: VecDeque::new(),
            undelivered: VecDeque::new(),
            taps: VecDeque::new(),
            progress: None,
            open_approval: None,
            last_received: None,
        }
    }

    /// Talks to a Bot API server other than Telegram's own.
    pub fn at(mut self, api_root: impl Into<String>) -> Self {
        self.bot_mut().api_root = api_root.into().trim_end_matches('/').to_owned();
        self
    }

    /// The language of the bot's own words: buttons, decisions, its menu.
    pub fn speaking(mut self, language: Language) -> Self {
        self.bot_mut().language = language;
        self
    }

    /// The folder where the files the Owner sends are saved.
    pub fn saving_files_in(mut self, inbox: impl Into<PathBuf>) -> Self {
        self.bot_mut().inbox = Some(inbox.into());
        self
    }

    /// How long to wait before asking again when Telegram cannot be reached.
    pub fn retrying_after(mut self, delay: Duration) -> Self {
        self.bot_mut().retry_after = delay;
        self
    }

    fn bot_mut(&mut self) -> &mut Bot {
        Arc::get_mut(&mut self.bot).expect("the Channel is set up before it is used")
    }

    /// The next thing the Owner sent; `None` once listening has ended.
    async fn next(&mut self) -> Option<Sent> {
        let bot = &self.bot;
        let (sent, _) = self.listening.get_or_insert_with(|| {
            let (sending, sent) = mpsc::unbounded_channel();
            (sent, tokio::spawn(listen(bot.clone(), sending)))
        });
        let received = sent.recv().await;
        if let Some(Sent::Text(_, message)) = &received {
            self.last_received = Some(*message);
        }
        received
    }

    /// Replaces the buttons of the Approval being asked by the decision, so
    /// that the Conversation keeps a record of what was allowed.
    async fn close_approval(&mut self, answer: &Answer, how: &str) {
        let Some((message, text)) = self.open_approval.take() else { return };
        let decision = match answer {
            Answer::Allowed => self.bot.language.say("✅ Allowed", "✅ Autorisé"),
            Answer::AlwaysAllowed => {
                self.bot.language.say("✅ Always allowed", "✅ Toujours autorisé")
            }
            Answer::Denied => self.bot.language.say("❌ Denied", "❌ Refusé"),
            Answer::Wrote(_) => self.bot.language.say("✍️ Not answered", "✍️ Sans réponse"),
        };
        let body = json!({
            "chat_id": self.bot.owner,
            "message_id": message,
            "text": format!("{text}\n\n{decision}{how}"),
            "parse_mode": "HTML",
        });
        let _ = self.bot.call::<Value>("editMessageText", body).await;
    }

    /// Removes the message showing the steps of the work, if there is one.
    async fn clear_progress(&mut self) {
        if let Some((message, _)) = self.progress.take() {
            let body = json!({ "chat_id": self.bot.owner, "message_id": message });
            let _ = self.bot.call::<Value>("deleteMessage", body).await;
        }
    }

    /// Tells Telegram a tap was received, so the button stops spinning.
    async fn acknowledge(&self, tap: &Tap, note: &str) {
        let body = json!({ "callback_query_id": tap.id, "text": note });
        let _ = self.bot.call::<Value>("answerCallbackQuery", body).await;
    }
}

impl Drop for Telegram {
    fn drop(&mut self) {
        if let Some((_, listening)) = &self.listening {
            listening.abort();
        }
    }
}

impl Bot {
    async fn call<T: DeserializeOwned>(&self, method: &str, body: Value) -> io::Result<T> {
        let response = self
            .http
            .post(format!("{}/bot{}/{method}", self.api_root, self.token))
            .timeout(LONG_POLL + Duration::from_secs(10))
            .json(&body)
            .send()
            .await
            .map_err(io::Error::other)?;
        let reply: Reply<T> = response.json().await.map_err(io::Error::other)?;
        match reply.result {
            Some(result) if reply.ok => Ok(result),
            _ => Err(io::Error::other(
                reply.description.unwrap_or_else(|| format!("Telegram refused {method}")),
            )),
        }
    }

    /// Fetches a file the Owner sent and saves it in the inbox, under a name
    /// that stays inside it and replaces nothing. The text explains a failure
    /// to the Owner.
    async fn fetch(&self, file_id: &str, name: &str, size: u64) -> Result<PathBuf, String> {
        let Some(inbox) = &self.inbox else {
            return Err("I have nowhere to keep files yet, so I can only read text.".into());
        };
        if size > FETCH_LIMIT {
            return Err(format!(
                "That file is too big for me to fetch: Telegram lets bots fetch files up to {} MB.",
                FETCH_LIMIT / 1_000_000
            ));
        }
        let failed =
            |error: &dyn std::fmt::Display| format!("I could not fetch that file ({error}).");
        let located: Located = self
            .call("getFile", json!({ "file_id": file_id }))
            .await
            .map_err(|error| failed(&error))?;
        let response = self
            .http
            .get(format!("{}/file/bot{}/{}", self.api_root, self.token, located.file_path))
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| failed(&error))?;
        let content = response.bytes().await.map_err(|error| failed(&error))?;

        // Only the last part of the name is kept, so it cannot lead elsewhere.
        let name = Path::new(name).file_name().and_then(|name| name.to_str()).unwrap_or("file");
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
            _ => (name, String::new()),
        };
        std::fs::create_dir_all(inbox).map_err(|error| failed(&error))?;
        let mut place = inbox.join(name);
        let mut copy = 2;
        while place.exists() {
            place = inbox.join(format!("{stem}-{copy}{extension}"));
            copy += 1;
        }
        std::fs::write(&place, &content).map_err(|error| failed(&error))?;
        Ok(place)
    }

    /// Sends the Owner `file` as a document, with `caption`.
    async fn upload(&self, file: &Path, caption: &str) -> io::Result<()> {
        let name = file.file_name().and_then(|name| name.to_str()).unwrap_or("file").to_owned();
        let content = std::fs::read(file)?;
        let form = reqwest::multipart::Form::new()
            .text("chat_id", self.owner.to_string())
            .text("caption", caption.to_owned())
            .part("document", reqwest::multipart::Part::bytes(content).file_name(name));
        let response = self
            .http
            .post(format!("{}/bot{}/sendDocument", self.api_root, self.token))
            .timeout(Duration::from_secs(300))
            .multipart(form)
            .send()
            .await
            .map_err(io::Error::other)?;
        let reply: Reply<Value> = response.json().await.map_err(io::Error::other)?;
        if reply.ok {
            Ok(())
        } else {
            Err(io::Error::other(
                reply.description.unwrap_or_else(|| "Telegram refused the file".into()),
            ))
        }
    }

    /// Sends `text` to the Owner, formatted, in as many messages as it takes.
    async fn say(&self, text: &str) -> io::Result<()> {
        for part in split(text, MESSAGE_LIMIT) {
            let plain = json!({ "chat_id": self.owner, "text": part });
            let html = formatting::to_html(&part);
            if !html.is_empty() && html.encode_utf16().count() <= MESSAGE_LIMIT {
                let formatted =
                    json!({ "chat_id": self.owner, "text": html, "parse_mode": "HTML" });
                if self.call::<Value>("sendMessage", formatted).await.is_ok() {
                    continue;
                }
            }
            // Delivering the answer matters more than its formatting.
            let _: Value = self.call("sendMessage", plain).await?;
        }
        Ok(())
    }
}

/// Asks Telegram for what was sent to the bot, again and again, and passes on
/// what the Owner sent. Ends when nobody is left to pass it on to.
async fn listen(bot: Arc<Bot>, sending: mpsc::UnboundedSender<Sent>) {
    // What the Owner can always type, offered in Telegram's own menu.
    let menu = json!({ "commands": [
        { "command": "stop", "description": bot.language.say("Stop what the agent is doing", "Arrêter ce que fait l'agent") },
        { "command": "reset", "description": bot.language.say(
            "Start the conversation afresh (memory is kept)",
            "Repartir de zéro dans la discussion (la mémoire est gardée)",
        ) },
        { "command": "usage", "description": bot.language.say("What the agent has used", "Ce que l'agent a consommé") },
        { "command": "settings", "description": bot.language.say("See and change the settings", "Voir et changer les réglages") },
        { "command": "mcp", "description": bot.language.say("List the MCP servers and their state", "Lister les serveurs MCP et leur état") },
        { "command": "extensions", "description": bot.language.say(
            "List the extensions, turn one off or remove it",
            "Lister les extensions, en désactiver ou en supprimer une",
        ) },
    ] });
    let _ = bot.call::<Value>("setMyCommands", menu).await;
    // Identifier of the next update to ask Telegram for.
    let mut next_update = 0;
    // How many times in a row Telegram could not be asked.
    let mut failures: u32 = 0;
    loop {
        let body = json!({
            "offset": next_update,
            "timeout": LONG_POLL.as_secs(),
            "allowed_updates": ["message", "callback_query"],
        });
        let updates: Vec<Update> = match bot.call("getUpdates", body).await {
            Ok(updates) => updates,
            Err(error) => {
                // Said once in the journal, then asked less and less often:
                // a token Telegram refuses does not start working by itself.
                if failures == 0 {
                    eprintln!(
                        "Telegram cannot be listened to ({error}). If it refuses the bot's token, \
                         enter a working one with `orbite setup telegram`. Trying again, less and \
                         less often."
                    );
                }
                let wait = bot.retry_after * 2u32.pow(failures.min(MOST_DOUBLINGS));
                failures += 1;
                tokio::time::sleep(wait).await;
                continue;
            }
        };
        if failures > 0 {
            eprintln!("Telegram is listened to again.");
            failures = 0;
        }
        for update in updates {
            next_update = update.update_id + 1;
            let sent = if let Some(tapped) = update.callback_query {
                // Only the Owner's taps count; anyone else's are dropped.
                match tapped.message {
                    Some(message) if tapped.from.id == bot.owner => Sent::Tap(Tap {
                        id: tapped.id,
                        message: message.message_id,
                        choice: tapped.data.unwrap_or_default(),
                    }),
                    _ => continue,
                }
            } else if let Some(message) = update.message {
                // Only the Owner, writing to the bot directly, is ever listened to.
                if message.from.map(|sender| sender.id) != Some(bot.owner)
                    || message.chat.id != bot.owner
                {
                    eprintln!("Telegram: ignored a message that is not from the Owner");
                    continue;
                }
                // A file, in the largest size Telegram offers of a photo.
                let file = match (message.document, message.photo.and_then(|mut sizes| sizes.pop()))
                {
                    (Some(document), _) => {
                        let name = document
                            .file_name
                            .unwrap_or_else(|| format!("file-{}", update.update_id));
                        Some((document.file_id, name, document.file_size.unwrap_or(0)))
                    }
                    (None, Some(photo)) => {
                        let name = format!("photo-{}.jpg", update.update_id);
                        Some((photo.file_id, name, photo.file_size.unwrap_or(0)))
                    }
                    (None, None) => None,
                };
                let words = message.text.or(message.caption).unwrap_or_default();
                match file {
                    None if words.is_empty() => {
                        let cannot = bot.language.say(
                            "I can only read text and files for now.",
                            "Je ne sais lire que du texte et des fichiers pour l'instant.",
                        );
                        let _ = bot.say(cannot).await;
                        continue;
                    }
                    None => Sent::Text(words, message.message_id),
                    Some((file_id, name, size)) => match bot.fetch(&file_id, &name, size).await {
                        Ok(saved) if words.is_empty() => {
                            Sent::Text(attached(&saved), message.message_id)
                        }
                        Ok(saved) => {
                            Sent::Text(format!("{words}\n{}", attached(&saved)), message.message_id)
                        }
                        Err(explanation) => {
                            let _ = bot.say(&explanation).await;
                            continue;
                        }
                    },
                }
            } else {
                continue;
            };
            if sending.send(sent).is_err() {
                return;
            }
        }
    }
}

impl Channel for Telegram {
    async fn receive(&mut self) -> Option<String> {
        loop {
            // No Approval is open here, so any tap is on an old one.
            while let Some(tap) = self.taps.pop_front() {
                // A choice offered earlier stands for the command it carries.
                if tap.choice.starts_with('/') {
                    self.acknowledge(&tap, "").await;
                    return Some(tap.choice);
                }
                self.acknowledge(&tap, "This request is no longer open.").await;
            }
            if let Some(text) = self.waiting.pop_front() {
                return Some(text);
            }
            match self.next().await? {
                Sent::Text(text, _) => return Some(text),
                Sent::Tap(tap) => self.taps.push_back(tap),
            }
        }
    }

    async fn send(&mut self, text: &str) -> io::Result<()> {
        self.clear_progress().await;
        // Telegram failing is not the Conversation ending: what it could not
        // take is kept, and goes out before whatever is sent next.
        self.undelivered.push_back(text.to_owned());
        while let Some(oldest) = self.undelivered.front() {
            if let Err(error) = self.bot.say(oldest).await {
                eprintln!(
                    "Telegram did not take a message ({error}): it is kept for the next try."
                );
                break;
            }
            self.undelivered.pop_front();
        }
        // No more is kept than an Owner would want to find on coming back.
        while self.undelivered.len() > UNDELIVERED_KEPT {
            self.undelivered.pop_front();
        }
        Ok(())
    }

    async fn offer(&mut self, text: &str, choices: &[(String, String)]) -> io::Result<()> {
        self.clear_progress().await;
        // One choice per line: readable and easy to tap on a phone.
        let buttons: Vec<Value> = choices
            .iter()
            .map(|(label, command)| json!([{ "text": label, "callback_data": command }]))
            .collect();
        let body = json!({
            "chat_id": self.bot.owner,
            "text": formatting::to_html(text),
            "parse_mode": "HTML",
            "reply_markup": { "inline_keyboard": buttons },
        });
        self.bot.call::<Value>("sendMessage", body).await.map(|_| ())
    }

    async fn send_file(&mut self, file: &Path, caption: &str) -> io::Result<()> {
        self.clear_progress().await;
        self.bot.upload(file, caption).await
    }

    async fn echo(&mut self, text: &str) {
        let body = json!({
            "chat_id": self.bot.owner,
            "text": format!(
                "🗣 <i>{}</i>\n{}",
                self.bot.language.say("You, elsewhere:", "Vous, ailleurs :"),
                formatting::escape(text)
            ),
            "parse_mode": "HTML",
            "disable_notification": true,
        });
        let _ = self.bot.call::<Value>("sendMessage", body).await;
    }

    async fn settle(&mut self, _approval: &Approval, answer: &Answer) {
        let elsewhere = self.bot.language.say(" (elsewhere)", " (ailleurs)");
        self.close_approval(answer, elsewhere).await;
    }

    async fn erase_last_received(&mut self) {
        if let Some(message) = self.last_received.take() {
            let body = json!({ "chat_id": self.bot.owner, "message_id": message });
            let _ = self.bot.call::<Value>("deleteMessage", body).await;
        }
    }

    async fn doing(&mut self, step: &Step) {
        let line = formatting::escape(&format!("{} {}", step.icon, step.text));
        let (message, mut steps) = self.progress.take().unzip();
        let steps = steps.get_or_insert_with(Vec::new);
        steps.push(line);
        // The step under way, with the few before it shown as done.
        let latest = &steps[steps.len().saturating_sub(STEPS_SHOWN)..];
        let (current, done) = latest.split_last().expect("a step was just added");
        let mut text: String = done.iter().map(|step| format!("<i>{step}</i>\n")).collect();
        text.push_str(&format!("{current}…"));
        let shown = match message {
            Some(message) => {
                let body = json!({
                    "chat_id": self.bot.owner, "message_id": message, "text": text, "parse_mode": "HTML",
                });
                let _ = self.bot.call::<Value>("editMessageText", body).await;
                Some(message)
            }
            None => {
                let body = json!({
                    "chat_id": self.bot.owner, "text": text, "parse_mode": "HTML",
                    "disable_notification": true,
                });
                self.bot.call::<Message>("sendMessage", body).await.ok().map(|sent| sent.message_id)
            }
        };
        self.progress = shown.map(|message| (message, std::mem::take(steps)));
    }

    async fn idle(&mut self) {
        self.clear_progress().await;
    }

    async fn ask(&mut self, approval: &Approval) -> io::Result<Answer> {
        self.clear_progress().await;
        // What is acted on stands out; the rest folds away when long.
        let (subject, rest) = approval.detail.split_once("\n\n").unwrap_or((&approval.detail, ""));
        let mut text = format!(
            "🛡 <b>{}</b>\n<code>{}</code>",
            formatting::escape(&approval.summary),
            formatting::escape(subject)
        );
        if !rest.trim().is_empty() {
            text.push_str(&format!(
                "\n<blockquote expandable>{}</blockquote>",
                formatting::escape(rest.trim())
            ));
        }
        if approval.irreversible {
            text.push_str("\n\n⚠️ ");
            text.push_str(
                self.bot.language.say("This cannot be undone.", "Cela ne peut pas être annulé."),
            );
        }
        let language = self.bot.language;
        let say = |english: &'static str, french: &'static str| language.say(english, french);
        let mut buttons = vec![
            json!({ "text": format!("✅ {}", say("Allow", "Autoriser")), "callback_data": "allow" }),
        ];
        if !approval.critical {
            buttons.push(json!({ "text": format!("🔁 {}", say("Always", "Toujours")), "callback_data": "always" }));
        }
        buttons.push(
            json!({ "text": format!("❌ {}", say("Deny", "Refuser")), "callback_data": "deny" }),
        );
        let asked: Message = self
            .bot
            .call(
                "sendMessage",
                json!({
                    "chat_id": self.bot.owner,
                    "text": text,
                    "parse_mode": "HTML",
                    "reply_markup": { "inline_keyboard": [buttons] },
                }),
            )
            .await?;

        self.open_approval = Some((asked.message_id, text.clone()));
        let answer = loop {
            let tap = match self.taps.pop_front() {
                Some(tap) => tap,
                None => match self.next().await {
                    Some(Sent::Tap(tap)) => tap,
                    // Writing instead of tapping is an answer of its own.
                    Some(Sent::Text(text, _)) => break Answer::Wrote(text),
                    None => return Err(io::Error::other("Telegram is no longer listened to")),
                },
            };
            if tap.message == asked.message_id {
                self.acknowledge(&tap, "").await;
                break match tap.choice.as_str() {
                    "allow" => Answer::Allowed,
                    "always" if !approval.critical => Answer::AlwaysAllowed,
                    _ => Answer::Denied,
                };
            }
            self.acknowledge(&tap, "This request is no longer open.").await;
        };

        self.close_approval(&answer, "").await;
        Ok(answer)
    }

    async fn interruption(&mut self) -> Option<String> {
        loop {
            if let Some(text) = self.waiting.pop_front() {
                return Some(text);
            }
            match self.next().await? {
                Sent::Text(text, _) => return Some(text),
                Sent::Tap(tap) => self.taps.push_back(tap),
            }
        }
    }

    async fn working(&mut self) {
        let body = json!({ "chat_id": self.bot.owner, "action": "typing" });
        let _ = self.bot.call::<Value>("sendChatAction", body).await;
    }
}

/// How many messages Telegram could not take are kept for a later try.
const UNDELIVERED_KEPT: usize = 20;

/// How many times the wait between two failed tries doubles: with the
/// usual three seconds, up to a little over three minutes.
const MOST_DOUBLINGS: u32 = 6;

#[derive(Deserialize)]
struct Reply<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct Update {
    update_id: i64,
    message: Option<Incoming>,
    callback_query: Option<Tapped>,
}

#[derive(Deserialize)]
struct Tapped {
    id: String,
    from: Account,
    message: Option<Message>,
    data: Option<String>,
}

/// A message of the bot's own.
#[derive(Deserialize)]
struct Message {
    message_id: i64,
}

#[derive(Deserialize)]
struct Incoming {
    message_id: i64,
    from: Option<Account>,
    chat: Account,
    text: Option<String>,
    caption: Option<String>,
    document: Option<Attached>,
    photo: Option<Vec<Attached>>,
}

/// A file as Telegram describes it.
#[derive(Deserialize)]
struct Attached {
    file_id: String,
    file_name: Option<String>,
    file_size: Option<u64>,
}

/// Where Telegram keeps a file to fetch.
#[derive(Deserialize)]
struct Located {
    file_path: String,
}

#[derive(Deserialize)]
struct Account {
    id: i64,
}

/// Cuts `text` into parts Telegram accepts, at line ends wherever possible.
/// `limit` counts UTF-16 units, as Telegram does.
fn split(text: &str, limit: usize) -> Vec<String> {
    fn finish(parts: &mut Vec<String>, part: &mut String) {
        let part = std::mem::take(part);
        let part = part.strip_suffix('\n').unwrap_or(&part);
        if !part.is_empty() {
            parts.push(part.to_owned());
        }
    }
    let length = |text: &str| text.encode_utf16().count();

    let mut parts = Vec::new();
    let mut part = String::new();
    for line in text.split_inclusive('\n') {
        if length(&part) + length(line) <= limit {
            part.push_str(line);
            continue;
        }
        finish(&mut parts, &mut part);
        // A single line over the limit has to be cut somewhere.
        for character in line.chars() {
            if length(&part) + character.len_utf16() > limit {
                parts.push(std::mem::take(&mut part));
            }
            part.push(character);
        }
    }
    finish(&mut parts, &mut part);
    parts
}
