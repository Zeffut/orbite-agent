//! The Conversation in a terminal: the same one the Owner holds on the
//! Agent's page and in its apps, on a screen of its own. It is a device
//! like the others, let in by the Agent it runs beside: what is said here
//! is said everywhere, and what was said elsewhere is here.

use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::welcome::{
    ACCENT, BOLD, PLAIN, QUIET, Row, WARN, byte, give_the_terminal_back, mascot, put, size,
    take_the_terminal, wrapped,
};
use crate::{Language, Look, Mood};

/// One thing of the Conversation, as the terminal shows it.
#[derive(Debug, Clone, PartialEq)]
enum Said {
    Owner(String),
    Agent(String),
    Ask { ask: String, summary: String, detail: String, critical: bool, irreversible: bool },
    Settled(String),
    File(String),
}

/// A key pressed.
#[derive(Debug, PartialEq, Eq)]
enum Key {
    Letter(char),
    Enter,
    NewLine,
    Erase,
    EraseAll,
    Left,
    Right,
    PageUp,
    PageDown,
    Leave,
    Other,
}

/// Everything the screen shows.
#[derive(Default)]
struct View {
    name: String,
    look: Option<Look>,
    said: Vec<(i64, Said)>,
    doing: Option<String>,
    working: bool,
    reached: bool,
    typed: String,
    /// How far up from the latest the Owner has gone, in lines.
    back: usize,
    /// Which answer is pointed at, when the Agent asks leave.
    answer: usize,
    note: Option<String>,
    french: bool,
}

impl View {
    fn say<'a>(&self, english: &'a str, french: &'a str) -> &'a str {
        if self.french { french } else { english }
    }

    /// The request the Agent waits on, when there is one.
    fn asked(&self) -> Option<(String, bool)> {
        self.said.iter().rev().find_map(|(_, said)| match said {
            Said::Ask { ask, critical, .. } => Some((ask.clone(), *critical)),
            _ => None,
        })
    }
}

/// What a line written by the Agent looks like in a terminal: its bold,
/// its code, and its links left as they are.
fn inline(line: &str, base: &str) -> (String, usize) {
    let mut out = String::from(base);
    let mut width = 0;
    let mut rest = line;
    while !rest.is_empty() {
        let bold = rest.find("**");
        let code = rest.find('`');
        let (at, mark, style) = match (bold, code) {
            (Some(bold), Some(code)) if bold < code => (bold, "**", BOLD),
            (Some(bold), None) => (bold, "**", BOLD),
            (_, Some(code)) => (code, "`", ACCENT),
            (None, None) => break,
        };
        let Some(length) = rest[at + mark.len()..].find(mark) else { break };
        let (before, inside) = (&rest[..at], &rest[at + mark.len()..at + mark.len() + length]);
        out.push_str(before);
        out.push_str(&format!("{style}{inside}{PLAIN}{base}"));
        width += before.chars().count() + inside.chars().count();
        rest = &rest[at + 2 * mark.len() + length..];
    }
    out.push_str(rest);
    out.push_str(PLAIN);
    (out, width + rest.chars().count())
}

/// What the Agent wrote, as rows: its lists, titles and code laid out.
fn written(text: &str, margin: usize, wide: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            // Kept on its line, cut where the terminal ends: code broken
            // in two reads as two lines of code.
            let room = wide.saturating_sub(2);
            let kept: String = if line.chars().count() > room {
                line.chars().take(room.saturating_sub(1)).chain(std::iter::once('…')).collect()
            } else {
                line.to_owned()
            };
            rows.push(Row::styled(margin + 2, QUIET, &kept));
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            rows.push(Row::blank());
            continue;
        }
        // A list's mark, and the room its text is set back by.
        let numbered = trimmed
            .split_once(['.', ')'])
            .filter(|(number, rest)| {
                !number.is_empty()
                    && number.chars().all(|digit| digit.is_ascii_digit())
                    && rest.starts_with(' ')
            })
            .map(|(number, rest)| (format!("{number}."), rest.trim_start()));
        let bulleted = ["- ", "* ", "• "]
            .iter()
            .find_map(|mark| trimmed.strip_prefix(mark))
            .map(|rest| ("•".to_owned(), rest));
        let title = trimmed.trim_start_matches('#');
        if let Some((mark, rest)) = bulleted.or(numbered) {
            let indent = mark.chars().count() + 1;
            for (place, part) in wrapped(rest, wide.saturating_sub(indent + 2)).iter().enumerate() {
                let (styled, width) = inline(part, "");
                let lead =
                    if place == 0 { format!("{QUIET}{mark}{PLAIN} ") } else { " ".repeat(indent) };
                rows.push(Row {
                    text: format!("{}{lead}{styled}", " ".repeat(margin + 2)),
                    width: margin + 2 + indent + width,
                });
            }
        } else if title.len() < trimmed.len() && title.starts_with(' ') {
            for part in wrapped(title.trim(), wide) {
                let (styled, width) = inline(&part, BOLD);
                rows.push(Row {
                    text: format!("{}{styled}", " ".repeat(margin)),
                    width: margin + width,
                });
            }
        } else {
            for part in wrapped(trimmed, wide) {
                let (styled, width) = inline(&part, "");
                rows.push(Row {
                    text: format!("{}{styled}", " ".repeat(margin)),
                    width: margin + width,
                });
            }
        }
    }
    rows
}

/// The Conversation as rows, the latest last.
fn conversation(view: &View, margin: usize, wide: usize) -> Vec<Row> {
    let colour = view.look.map_or(255, |look| look.in_a_terminal());
    let mut rows = Vec::new();
    for (_, said) in &view.said {
        match said {
            Said::Owner(text) => {
                rows.push(Row::styled(margin, QUIET, view.say("you", "toi")));
                rows.extend(wrapped(text, wide).iter().map(|line| Row::of(margin, line)));
            }
            Said::Agent(text) => {
                rows.push(Row::pieces(
                    margin,
                    &[(&format!("\x1b[38;5;{colour}m"), "● "), (BOLD, &view.name)],
                ));
                rows.extend(written(text, margin, wide));
                // A link that lets a device in, as a code to point a phone at.
                let link = text
                    .split_whitespace()
                    .find(|word| word.starts_with("http") && word.contains("/#"))
                    .filter(|link| {
                        !["http://127.", "http://localhost"].iter().any(|own| link.starts_with(own))
                    });
                if let Some(link) = link {
                    rows.push(Row::blank());
                    rows.extend(crate::welcome::square_code(link).into_iter().map(|row| Row {
                        text: format!("{}{}", " ".repeat(margin), row.text),
                        width: margin + row.width,
                    }));
                }
            }
            Said::Ask { summary, detail, critical, irreversible, ask } => {
                let waited_on = view.asked().is_some_and(|(asked, _)| asked == *ask);
                let edge = if *critical { WARN } else { ACCENT };
                rows.push(Row::pieces(margin, &[(edge, "▍ "), (BOLD, summary)]));
                for line in wrapped(detail, wide.saturating_sub(2)) {
                    rows.push(Row::pieces(margin, &[(edge, "▍ "), (QUIET, &line)]));
                }
                if *irreversible {
                    let undone =
                        view.say("This cannot be undone.", "Cela ne peut pas être annulé.");
                    rows.push(Row::pieces(margin, &[(edge, "▍ "), (WARN, undone)]));
                }
                if waited_on {
                    let offered: Vec<&str> = if *critical {
                        vec![view.say("Allow", "Autoriser"), view.say("Deny", "Refuser")]
                    } else {
                        vec![
                            view.say("Allow", "Autoriser"),
                            view.say("Always", "Toujours"),
                            view.say("Deny", "Refuser"),
                        ]
                    };
                    let mut choices = Row::pieces(margin, &[(edge, "▍ ")]);
                    for (place, label) in offered.iter().enumerate() {
                        let style = if place == view.answer { "\x1b[7m" } else { QUIET };
                        choices.text.push_str(&format!("{style} {label} {PLAIN}  "));
                        choices.width += label.chars().count() + 4;
                    }
                    rows.push(choices);
                }
            }
            Said::Settled(told) => rows.push(Row::styled(margin, QUIET, told)),
            Said::File(name) => {
                rows.push(Row::pieces(margin, &[(QUIET, "▫ "), ("", name)]));
            }
        }
        rows.push(Row::blank());
    }
    rows
}

/// The whole screen, for a terminal of `columns` by `lines`.
fn frame(view: &View, columns: usize, lines: usize, tick: u64) -> Vec<Row> {
    let wide = columns.saturating_sub(4).clamp(20, 84);
    let margin = columns.saturating_sub(wide) / 2;
    let colour = view.look.map_or(255, |look| look.in_a_terminal());
    // The top: who this is, and what it is doing.
    const TURNING: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut top =
        Row::pieces(margin, &[(&format!("\x1b[38;5;{colour}m"), "● "), (BOLD, &view.name)]);
    let state = match (&view.doing, view.working, view.reached) {
        (_, _, false) => Some((WARN, view.say("not reached", "injoignable").to_owned())),
        (Some(doing), _, _) => Some((QUIET, format!("{} {doing}", TURNING[(tick % 10) as usize]))),
        (None, true, _) => Some((
            QUIET,
            format!("{} {}", TURNING[(tick % 10) as usize], view.say("working", "au travail")),
        )),
        _ => None,
    };
    if let Some((style, state)) = state {
        let shown: String =
            state.chars().take(wide.saturating_sub(view.name.chars().count() + 6)).collect();
        top.text.push_str(&format!("  {style}{shown}{PLAIN}"));
        top.width += 2 + shown.chars().count();
    }
    // The bottom: what is being typed, and what the keys do.
    let asked = view.asked();
    let mut bottom = Vec::new();
    if let Some(note) = &view.note {
        bottom.push(Row::styled(margin, WARN, note));
    }
    let typed: Vec<String> = wrapped(&format!("{}▏", view.typed), wide.saturating_sub(2));
    let from = typed.len().saturating_sub(6);
    for (place, line) in typed[from..].iter().enumerate() {
        let lead = if place == 0 { "❯ " } else { "  " };
        bottom.push(Row::pieces(margin, &[(ACCENT, lead), ("", line)]));
    }
    let hint = match (asked, view.typed.is_empty()) {
        (Some(_), true) => view.say(
            "←→ choose · Enter answer · or write · Esc leave",
            "←→ choisir · Entrée répondre · ou écris · Échap quitter",
        ),
        _ => view.say(
            "Enter send · Alt-Enter new line · PgUp PgDn scroll · Esc leave",
            "Entrée envoyer · Alt-Entrée à la ligne · PgUp PgDn défiler · Échap quitter",
        ),
    };
    let shown: String = hint.chars().take(wide).collect();
    bottom.push(Row::styled(margin, QUIET, &shown));

    let mut rows = vec![top, Row::blank()];
    let room = lines.saturating_sub(rows.len() + bottom.len() + 1).max(1);
    let all = conversation(view, margin, wide);
    if all.is_empty() {
        // Nothing said yet: the Agent, waiting.
        let mood = if view.working { Mood::Working } else { Mood::Waiting };
        let figure = mascot(view.look, mood, tick);
        let above = room.saturating_sub(figure.len() + 2) / 2;
        rows.extend(std::iter::repeat_n(Row::blank(), above));
        for row in &figure {
            let indent = columns.saturating_sub(row.width) / 2;
            rows.push(Row {
                text: format!("{}{}", " ".repeat(indent), row.text),
                width: indent + row.width,
            });
        }
        let greeting = view.say("Say something to", "Dis quelque chose à");
        let line = format!("{greeting} {}", view.name);
        rows.push(Row::blank());
        rows.push(Row::styled(columns.saturating_sub(line.chars().count()) / 2, QUIET, &line));
        rows.extend(std::iter::repeat_n(
            Row::blank(),
            room.saturating_sub(above + figure.len() + 2),
        ));
    } else {
        // As much of the end as there is room for, or further back.
        let back = view.back.min(all.len().saturating_sub(room));
        let until = all.len() - back;
        let since = until.saturating_sub(room);
        rows.extend(std::iter::repeat_n(Row::blank(), room - (until - since)));
        rows.extend(all[since..until].iter().cloned());
        if back > 0 {
            let more = view.say("↓ more below", "↓ la suite plus bas");
            rows.pop();
            rows.push(Row::styled(margin, ACCENT, more));
        }
    }
    rows.push(Row::blank());
    rows.extend(bottom);
    rows
}

/// The next key pressed, or none when the terminal went away.
fn key() -> Option<Key> {
    let first = byte(None)?;
    Some(match first {
        3 | 4 => Key::Leave,
        b'\r' => Key::Enter,
        b'\n' => Key::NewLine,
        0x7f | 0x08 => Key::Erase,
        0x15 => Key::EraseAll,
        0x1b => match byte(Some(40)) {
            // Escape alone leaves; Alt with Enter goes to a new line.
            None => Key::Leave,
            Some(b'\r' | b'\n') => Key::NewLine,
            Some(b'[' | b'O') => {
                let mut sequence = Vec::new();
                while let Some(next) = byte(Some(40)) {
                    sequence.push(next);
                    if (0x40..=0x7e).contains(&next) {
                        break;
                    }
                }
                match sequence.as_slice() {
                    [b'C'] => Key::Right,
                    [b'D'] => Key::Left,
                    [b'5', b'~'] => Key::PageUp,
                    [b'6', b'~'] => Key::PageDown,
                    _ => Key::Other,
                }
            }
            Some(_) => Key::Other,
        },
        first if first < 0x20 => Key::Other,
        first => {
            let more = match first {
                0xc0..=0xdf => 1,
                0xe0..=0xef => 2,
                0xf0..=0xf7 => 3,
                _ => 0,
            };
            let mut bytes = vec![first];
            for _ in 0..more {
                bytes.extend(byte(Some(40)));
            }
            match std::str::from_utf8(&bytes).ok().and_then(|text| text.chars().next()) {
                Some(letter) => Key::Letter(letter),
                None => Key::Other,
            }
        }
    })
}

/// The Agent's interface, as this terminal reaches it.
struct Reached {
    http: reqwest::Client,
    at: String,
    device: String,
}

impl Reached {
    fn asking(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.at))
            .header("x-orbite", "1")
            .header("origin", &self.at)
            .header("cookie", format!("orbite_device={}", self.device))
    }

    async fn post(&self, path: &str, body: Value) -> bool {
        let sent = self.asking(reqwest::Method::POST, path).json(&body).send().await;
        sent.is_ok_and(|answer| answer.status().is_success())
    }
}

/// Lets this terminal into the Conversation: with what it was let in by
/// before, or with a code made on the spot, as the Host's own may.
async fn let_in(home: &Path, address: SocketAddr) -> io::Result<Reached> {
    let failed = |why: String| io::Error::other(why);
    let http = reqwest::Client::builder().build().map_err(|error| failed(error.to_string()))?;
    let at = format!("http://{address}");
    let kept = home.join("terminal.device");
    if let Ok(device) = std::fs::read_to_string(&kept) {
        let reached =
            Reached { http: http.clone(), at: at.clone(), device: device.trim().to_owned() };
        let known = reached.asking(reqwest::Method::GET, "/api/me").send().await;
        match known {
            Ok(answer) if answer.status().is_success() => return Ok(reached),
            Ok(_) => {}
            Err(error) => return Err(failed(error.without_url().to_string())),
        }
    }
    let code = crate::pairing_code(home)?;
    let paired = http
        .post(format!("{at}/api/pair"))
        .header("x-orbite", "1")
        .header("origin", &at)
        .json(&json!({ "code": code, "name": "terminal" }))
        .send()
        .await
        .map_err(|error| failed(error.without_url().to_string()))?;
    let device = paired
        .headers()
        .get("set-cookie")
        .and_then(|cookie| cookie.to_str().ok())
        .and_then(|cookie| cookie.split(';').next()?.strip_prefix("orbite_device="))
        .ok_or_else(|| failed("the Agent did not let this terminal in".into()))?
        .to_owned();
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::remove_file(&kept);
        let mut written =
            std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&kept)?;
        written.write_all(device.as_bytes())?;
    }
    Ok(Reached { http, at, device })
}

/// What happens while the Conversation is on screen.
enum Happened {
    Key(Key),
    Event(Value),
    Reached(bool),
    Tick,
}

/// Holds the Conversation of the installation in `home`, whose interface
/// is served at `address`, in this terminal, until the Owner leaves.
pub async fn talk(home: &Path, address: SocketAddr, language: Language) -> io::Result<()> {
    let reached = std::sync::Arc::new(let_in(home, address).await?);
    let me: Value = match reached.asking(reqwest::Method::GET, "/api/me").send().await {
        Ok(answer) => answer.json().await.unwrap_or_default(),
        Err(_) => Value::Null,
    };
    let mut view = View {
        name: me["agent"].as_str().unwrap_or("Orbite").to_owned(),
        look: me["look"]["shape"].as_str().map(|shape| {
            let colour = crate::COLOURS
                .iter()
                .find(|(_, page, _)| Some(*page) == me["look"]["colour"].as_str())
                .map(|(name, ..)| *name);
            Look::called(Some(shape), colour)
        }),
        reached: true,
        french: language == Language::French,
        ..View::default()
    };

    let (happening, mut happened) = mpsc::unbounded_channel();
    // The keys, read where reading may wait.
    std::thread::spawn({
        let happening = happening.clone();
        move || {
            while let Some(pressed) = key() {
                if happening.send(Happened::Key(pressed)).is_err() {
                    return;
                }
            }
            let _ = happening.send(Happened::Key(Key::Leave));
        }
    });
    // What the Agent says, taken up again whenever the line drops.
    tokio::spawn({
        let (happening, reached) = (happening.clone(), reached.clone());
        async move {
            loop {
                if let Ok(mut answer) =
                    reached.asking(reqwest::Method::GET, "/api/events").send().await
                {
                    let _ = happening.send(Happened::Reached(true));
                    let mut waiting = String::new();
                    while let Ok(Some(part)) = answer.chunk().await {
                        waiting.push_str(&String::from_utf8_lossy(&part));
                        while let Some(end) = waiting.find('\n') {
                            let line: String = waiting.drain(..=end).collect();
                            if let Some(said) = line.trim().strip_prefix("data:")
                                && let Ok(event) = serde_json::from_str::<Value>(said.trim())
                                && happening.send(Happened::Event(event)).is_err()
                            {
                                return;
                            }
                        }
                    }
                }
                if happening.send(Happened::Reached(false)).is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    });
    tokio::spawn({
        let happening = happening.clone();
        async move {
            let mut every = tokio::time::interval(Duration::from_millis(100));
            loop {
                every.tick().await;
                if happening.send(Happened::Tick).is_err() {
                    return;
                }
            }
        }
    });

    let as_it_was = take_the_terminal()?;
    let mut drawn: Vec<Row> = Vec::new();
    let mut drawn_for = (0, 0);
    let mut tick = 0u64;
    let mut replaying = true;
    while let Some(what) = happened.recv().await {
        match what {
            Happened::Tick => tick += 1,
            Happened::Reached(now) => {
                view.reached = now;
                if now {
                    // The Conversation is told again from its start.
                    view.said.clear();
                    replaying = true;
                }
            }
            Happened::Event(event) => {
                let id = event["id"].as_i64().unwrap_or(-1);
                let text = || clean(event["text"].as_str().unwrap_or_default());
                match event["kind"].as_str() {
                    Some("owner") => {
                        let files = event["files"].as_array().map_or(0, Vec::len);
                        let said = match (text(), files) {
                            (said, 0) => said,
                            (said, files) => format!("{said} ▫ ×{files}").trim().to_owned(),
                        };
                        view.said.push((id, Said::Owner(said)));
                    }
                    Some("agent") => {
                        view.said.push((id, Said::Agent(text())));
                        view.working = false;
                        view.doing = None;
                    }
                    Some("ask") => {
                        view.answer = 0;
                        view.said.push((
                            id,
                            Said::Ask {
                                ask: event["ask"].as_str().unwrap_or_default().to_owned(),
                                summary: clean(event["summary"].as_str().unwrap_or_default()),
                                detail: clean(event["detail"].as_str().unwrap_or_default()),
                                critical: event["critical"] == true,
                                irreversible: event["irreversible"] == true,
                            },
                        ));
                    }
                    Some("settled") => {
                        let settled = event["ask"].as_str();
                        let told = match event["answer"].as_str() {
                            Some("allow") => view.say("allowed", "autorisé"),
                            Some("always") => view.say("always allowed", "toujours autorisé"),
                            Some("deny") => view.say("denied", "refusé"),
                            _ => view.say("answered", "répondu"),
                        }
                        .to_owned();
                        for (_, said) in &mut view.said {
                            if let Said::Ask { ask, summary, .. } = said
                                && Some(ask.as_str()) == settled
                            {
                                *said = Said::Settled(format!("{summary} · {told}"));
                            }
                        }
                    }
                    Some("file") => {
                        let name = clean(event["name"].as_str().unwrap_or_default());
                        let caption = clean(event["caption"].as_str().unwrap_or_default());
                        view.said
                            .push((id, Said::File(format!("{name} {caption}").trim().to_owned())));
                    }
                    Some("erase") => {
                        let gone: Vec<i64> = event["events"]
                            .as_array()
                            .map(|gone| gone.iter().filter_map(Value::as_i64).collect())
                            .unwrap_or_default();
                        view.said.retain(|(id, _)| !gone.contains(id));
                    }
                    Some("caught-up") => replaying = false,
                    Some("step") if !replaying => {
                        view.doing = Some(clean(&format!(
                            "{} {}",
                            event["icon"].as_str().unwrap_or_default(),
                            event["text"].as_str().unwrap_or_default()
                        )));
                        view.working = true;
                    }
                    Some("working") if !replaying => view.working = true,
                    Some("idle") => {
                        view.working = false;
                        view.doing = None;
                    }
                    _ => {}
                }
            }
            Happened::Key(pressed) => {
                view.note = None;
                let asked = view.asked();
                match pressed {
                    Key::Leave => break,
                    Key::Letter(letter) => view.typed.push(letter),
                    Key::NewLine => view.typed.push('\n'),
                    Key::Erase => {
                        view.typed.pop();
                    }
                    Key::EraseAll => view.typed.clear(),
                    Key::PageUp => view.back += 8,
                    Key::PageDown => view.back = view.back.saturating_sub(8),
                    Key::Left => view.answer = view.answer.saturating_sub(1),
                    Key::Right => {
                        let most = if asked.as_ref().is_some_and(|(_, critical)| *critical) {
                            1
                        } else {
                            2
                        };
                        view.answer = (view.answer + 1).min(most);
                    }
                    Key::Enter => {
                        view.back = 0;
                        let saying = view.typed.trim().to_owned();
                        let sent = match (asked, saying.is_empty()) {
                            // The answer pointed at, to what the Agent asked.
                            (Some((ask, critical)), true) => {
                                let answers: &[&str] = if critical {
                                    &["allow", "deny"]
                                } else {
                                    &["allow", "always", "deny"]
                                };
                                let answer = answers[view.answer.min(answers.len() - 1)];
                                reached
                                    .post("/api/answer", json!({ "ask": ask, "answer": answer }))
                                    .await
                            }
                            (_, true) => true,
                            (_, false) => {
                                let sent = reached
                                    .post("/api/say", json!({ "text": saying, "files": [] }))
                                    .await;
                                if sent {
                                    view.typed.clear();
                                }
                                sent
                            }
                        };
                        if !sent {
                            view.note = Some(
                                view.say(
                                    "That was not sent. Try again.",
                                    "Ce n'est pas parti. Réessaie.",
                                )
                                .to_owned(),
                            );
                        }
                    }
                    Key::Other => {}
                }
            }
        }
        // Drawn again, only the lines that changed.
        let (columns, lines) = size();
        let rows = frame(&view, columns, lines, tick);
        let mut out = String::new();
        if drawn_for != (columns, lines) {
            out.push_str("\x1b[2J");
            drawn.clear();
            drawn_for = (columns, lines);
        }
        for (place, row) in rows.iter().enumerate().take(lines) {
            if drawn.get(place) != Some(row) {
                out.push_str(&format!("\x1b[{};1H{}\x1b[K", place + 1, row.text));
            }
        }
        if drawn.len() > rows.len() {
            out.push_str(&format!("\x1b[{};1H\x1b[J", rows.len() + 1));
        }
        if !out.is_empty() {
            put(&out);
        }
        drawn = rows;
    }
    give_the_terminal_back(&as_it_was);
    Ok(())
}

/// `text` without what a terminal would obey rather than show.
fn clean(text: &str) -> String {
    crate::front::clean(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letters(rows: &[Row]) -> String {
        let mut plain = String::new();
        for row in rows {
            let mut colouring = false;
            for letter in row.text.chars() {
                match (colouring, letter) {
                    (false, '\x1b') => colouring = true,
                    (true, 'm') => colouring = false,
                    (false, letter) => plain.push(letter),
                    _ => {}
                }
            }
            plain.push('\n');
        }
        plain
    }

    fn talking(said: Vec<Said>) -> View {
        View {
            name: "Nova".into(),
            reached: true,
            said: said.into_iter().enumerate().map(|(place, said)| (place as i64, said)).collect(),
            ..View::default()
        }
    }

    #[test]
    fn what_the_agent_wrote_is_laid_out_with_its_lists_and_titles() {
        let view = talking(vec![Said::Agent(
            "## Plan\n- **First** thing\n- second\n1. one\n2. two\n```\ncode here\n```".into(),
        )]);
        let shown = letters(&frame(&view, 80, 30, 0));
        assert!(shown.contains("Plan") && !shown.contains("##"), "{shown}");
        assert!(shown.contains("• First thing") && !shown.contains("**"), "{shown}");
        assert!(shown.contains("1. one") && shown.contains("code here"), "{shown}");
    }

    #[test]
    fn a_link_that_lets_a_device_in_is_drawn_as_a_code_to_point_a_phone_at() {
        let pairing =
            "On the device, open https://host.example.ts.net:8377/#ABCD-EFGH\n\nIt works once.";
        let shown = letters(&frame(&talking(vec![Said::Agent(pairing.into())]), 100, 50, 0));
        assert!(shown.contains("█▀▀▀▀▀█"), "{shown}");
        // One this machine alone can follow is of no use to a phone.
        let local = "On the device, open http://127.0.0.1:8377/#ABCD-EFGH";
        let shown = letters(&frame(&talking(vec![Said::Agent(local.into())]), 100, 50, 0));
        assert!(!shown.contains("█▀▀▀▀▀█"), "{shown}");
    }

    #[test]
    fn the_screen_never_takes_more_lines_than_the_terminal_has() {
        let long = (0..80).map(|place| format!("line {place}")).collect::<Vec<_>>().join("\n");
        let mut view = talking(vec![Said::Owner("hello".into()), Said::Agent(long)]);
        view.typed = "a\nb\nc".into();
        for (columns, lines) in [(80, 24), (40, 12), (120, 50)] {
            let rows = frame(&view, columns, lines, 0);
            assert!(rows.len() <= lines, "{} rows on {lines} lines", rows.len());
            assert!(letters(&rows).contains("line 79"), "the latest is in view");
        }
        // Gone back up, the latest gives way and the way back is said.
        view.back = 16;
        let shown = letters(&frame(&view, 80, 24, 0));
        assert!(!shown.contains("line 79") && shown.contains("more below"), "{shown}");
    }

    #[test]
    fn a_request_is_answered_with_the_arrows_and_a_critical_one_never_always() {
        let ask = |critical| Said::Ask {
            ask: "4".into(),
            summary: "Run a command".into(),
            detail: "rm notes.txt".into(),
            critical,
            irreversible: true,
        };
        let shown = letters(&frame(&talking(vec![ask(false)]), 80, 24, 0));
        assert!(
            shown.contains("Allow") && shown.contains("Always") && shown.contains("Deny"),
            "{shown}"
        );
        assert!(shown.contains("cannot be undone"), "{shown}");
        let shown = letters(&frame(&talking(vec![ask(true)]), 80, 24, 0));
        assert!(shown.contains("Allow") && !shown.contains("Always"), "{shown}");
    }
}
