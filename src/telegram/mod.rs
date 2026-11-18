mod formatting;

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{Approval, Channel};

/// How long Telegram may hold a request open waiting for a message.
const LONG_POLL: Duration = Duration::from_secs(50);

/// The longest text Telegram accepts in one message, in UTF-16 units.
const MESSAGE_LIMIT: usize = 4096;

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
    /// Taps on Approval buttons not yet matched to an Approval.
    taps: VecDeque<Tap>,
}

/// How to reach Telegram as the bot.
struct Bot {
    http: reqwest::Client,
    api_root: String,
    token: String,
    owner: i64,
    retry_after: Duration,
}

/// Something the Owner sent to the bot.
enum Sent {
    Text(String),
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
            }),
            listening: None,
            waiting: VecDeque::new(),
            taps: VecDeque::new(),
        }
    }

    /// Talks to a Bot API server other than Telegram's own.
    pub fn at(mut self, api_root: impl Into<String>) -> Self {
        self.bot_mut().api_root = api_root.into().trim_end_matches('/').to_owned();
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
        sent.recv().await
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
    // Identifier of the next update to ask Telegram for.
    let mut next_update = 0;
    loop {
        let body = json!({
            "offset": next_update,
            "timeout": LONG_POLL.as_secs(),
            "allowed_updates": ["message", "callback_query"],
        });
        let updates: Vec<Update> = match bot.call("getUpdates", body).await {
            Ok(updates) => updates,
            Err(_) => {
                tokio::time::sleep(bot.retry_after).await;
                continue;
            }
        };
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
                match message.text {
                    Some(text) => Sent::Text(text),
                    None => {
                        let _ = bot.say("I can only read text for now.").await;
                        continue;
                    }
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
                self.acknowledge(&tap, "This request is no longer open.").await;
            }
            if let Some(text) = self.waiting.pop_front() {
                return Some(text);
            }
            match self.next().await? {
                Sent::Text(text) => return Some(text),
                Sent::Tap(tap) => self.taps.push_back(tap),
            }
        }
    }

    async fn send(&mut self, text: &str) -> io::Result<()> {
        self.bot.say(text).await
    }

    async fn ask(&mut self, approval: &Approval) -> io::Result<bool> {
        let mut text = format!(
            "<b>{}</b>\n\n<pre>{}</pre>",
            formatting::escape(&approval.summary),
            formatting::escape(&approval.detail)
        );
        if approval.irreversible {
            text.push_str("\n\n⚠️ This cannot be undone.");
        }
        let asked: Message = self
            .bot
            .call(
                "sendMessage",
                json!({
                    "chat_id": self.bot.owner,
                    "text": text,
                    "parse_mode": "HTML",
                    "reply_markup": { "inline_keyboard": [[
                        { "text": "Allow", "callback_data": "allow" },
                        { "text": "Deny", "callback_data": "deny" },
                    ]] },
                }),
            )
            .await?;

        let allowed = loop {
            let tap = match self.taps.pop_front() {
                Some(tap) => tap,
                None => match self.next().await {
                    Some(Sent::Tap(tap)) => tap,
                    // What the Owner writes meanwhile is read afterwards.
                    Some(Sent::Text(text)) => {
                        self.waiting.push_back(text);
                        continue;
                    }
                    None => return Err(io::Error::other("Telegram is no longer listened to")),
                },
            };
            if tap.message == asked.message_id {
                self.acknowledge(&tap, "").await;
                break tap.choice == "allow";
            }
            self.acknowledge(&tap, "This request is no longer open.").await;
        };

        // The buttons give way to the decision, so the Conversation keeps a
        // record of what was allowed.
        let decision = if allowed { "✅ Allowed" } else { "❌ Denied" };
        let _ = self
            .bot
            .call::<Value>(
                "editMessageText",
                json!({
                    "chat_id": self.bot.owner,
                    "message_id": asked.message_id,
                    "text": format!("{text}\n\n{decision}"),
                    "parse_mode": "HTML",
                }),
            )
            .await;
        Ok(allowed)
    }

    async fn interruption(&mut self) -> Option<String> {
        loop {
            if let Some(text) = self.waiting.pop_front() {
                return Some(text);
            }
            match self.next().await? {
                Sent::Text(text) => return Some(text),
                Sent::Tap(tap) => self.taps.push_back(tap),
            }
        }
    }

    async fn working(&mut self) {
        let body = json!({ "chat_id": self.bot.owner, "action": "typing" });
        let _ = self.bot.call::<Value>("sendChatAction", body).await;
    }
}

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
    from: Option<Account>,
    chat: Account,
    text: Option<String>,
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
