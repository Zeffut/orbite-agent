mod formatting;

use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{Approval, Channel};

/// How long Telegram may hold a request open waiting for a message.
const LONG_POLL: Duration = Duration::from_secs(50);

/// The longest text Telegram accepts in one message, in UTF-16 units.
const MESSAGE_LIMIT: usize = 4096;

/// The Conversation in Telegram, through a bot the Owner created.
///
/// The bot asks Telegram for new messages (long polling), so the Host needs no
/// open port, domain or certificate.
pub struct Telegram {
    http: reqwest::Client,
    api_root: String,
    token: String,
    owner: i64,
    retry_after: Duration,
    /// Identifier of the next update to ask Telegram for.
    next_update: i64,
    waiting: VecDeque<String>,
    /// Taps on Approval buttons not yet matched to an Approval.
    taps: VecDeque<Tap>,
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
            http: reqwest::Client::new(),
            api_root: "https://api.telegram.org".into(),
            token: token.into(),
            owner,
            retry_after: Duration::from_secs(3),
            next_update: 0,
            waiting: VecDeque::new(),
            taps: VecDeque::new(),
        }
    }

    /// Talks to a Bot API server other than Telegram's own.
    pub fn at(mut self, api_root: impl Into<String>) -> Self {
        self.api_root = api_root.into().trim_end_matches('/').to_owned();
        self
    }

    /// How long to wait before asking again when Telegram cannot be reached.
    pub fn retrying_after(mut self, delay: Duration) -> Self {
        self.retry_after = delay;
        self
    }

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

    /// Tells Telegram a tap was received, so the button stops spinning.
    async fn acknowledge(&self, tap: &Tap, note: &str) {
        let body = json!({ "callback_query_id": tap.id, "text": note });
        let _ = self.call::<Value>("answerCallbackQuery", body).await;
    }

    /// Asks Telegram for new messages and keeps the Owner's.
    async fn fetch(&mut self) -> io::Result<()> {
        let body = json!({
            "offset": self.next_update,
            "timeout": LONG_POLL.as_secs(),
            "allowed_updates": ["message", "callback_query"],
        });
        let updates: Vec<Update> = self.call("getUpdates", body).await?;
        for update in updates {
            self.next_update = update.update_id + 1;
            if let Some(tapped) = update.callback_query {
                // Only the Owner's taps count; anyone else's are dropped.
                if tapped.from.id == self.owner
                    && let Some(message) = tapped.message
                {
                    self.taps.push_back(Tap {
                        id: tapped.id,
                        message: message.message_id,
                        choice: tapped.data.unwrap_or_default(),
                    });
                }
                continue;
            }
            let Some(message) = update.message else {
                continue;
            };
            // Only the Owner, writing to the bot directly, is ever listened to.
            if message.from.map(|sender| sender.id) != Some(self.owner)
                || message.chat.id != self.owner
            {
                eprintln!("Telegram: ignored a message that is not from the Owner");
                continue;
            }
            match message.text {
                Some(text) => self.waiting.push_back(text),
                None => {
                    let _ = self.send("I can only read text for now.").await;
                }
            }
        }
        Ok(())
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
            if self.fetch().await.is_err() {
                tokio::time::sleep(self.retry_after).await;
            }
        }
    }

    async fn send(&mut self, text: &str) -> io::Result<()> {
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

    async fn ask(&mut self, approval: &Approval) -> io::Result<bool> {
        let mut text = format!(
            "<b>{}</b>\n\n<pre>{}</pre>",
            formatting::escape(&approval.summary),
            formatting::escape(&approval.detail)
        );
        if approval.irreversible {
            text.push_str("\n\n⚠️ This cannot be undone.");
        }
        let asked: Sent = self
            .call(
                "sendMessage",
                json!({
                    "chat_id": self.owner,
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
            match self.taps.pop_front() {
                Some(tap) if tap.message == asked.message_id => {
                    self.acknowledge(&tap, "").await;
                    break tap.choice == "allow";
                }
                Some(tap) => self.acknowledge(&tap, "This request is no longer open.").await,
                None => {
                    if self.fetch().await.is_err() {
                        tokio::time::sleep(self.retry_after).await;
                    }
                }
            }
        };

        // The buttons give way to the decision, so the Conversation keeps a
        // record of what was allowed.
        let decision = if allowed { "✅ Allowed" } else { "❌ Denied" };
        let _ = self
            .call::<Value>(
                "editMessageText",
                json!({
                    "chat_id": self.owner,
                    "message_id": asked.message_id,
                    "text": format!("{text}\n\n{decision}"),
                    "parse_mode": "HTML",
                }),
            )
            .await;
        Ok(allowed)
    }

    async fn working(&mut self) {
        let body = json!({ "chat_id": self.owner, "action": "typing" });
        let _ = self.call::<Value>("sendChatAction", body).await;
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
    message: Option<Sent>,
    data: Option<String>,
}

/// A message of the bot's own.
#[derive(Deserialize)]
struct Sent {
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
