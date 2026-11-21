use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, Lines};

use crate::{Answer, Approval, Channel};

/// A Channel for developers: the Conversation typed and read in a terminal.
pub struct Terminal<R, W> {
    typed: Lines<R>,
    shown: W,
    /// True while a prompt is on screen with nothing typed after it yet.
    prompted: bool,
    /// What was typed while the Agent worked and was not the order to
    /// stop: read in its turn, once the Agent listens again.
    typed_ahead: std::collections::VecDeque<String>,
}

impl<R: AsyncBufRead + Unpin + Send, W: AsyncWrite + Unpin + Send> Terminal<R, W> {
    pub fn new(typed: R, shown: W) -> Self {
        Self {
            typed: typed.lines(),
            shown,
            prompted: false,
            typed_ahead: std::collections::VecDeque::new(),
        }
    }

    async fn show(&mut self, text: &str) -> io::Result<()> {
        self.shown.write_all(text.as_bytes()).await?;
        self.shown.flush().await
    }
}

impl<R: AsyncBufRead + Unpin + Send, W: AsyncWrite + Unpin + Send> Channel for Terminal<R, W> {
    async fn receive(&mut self) -> Option<String> {
        if !self.prompted {
            self.show("> ").await.ok()?;
            self.prompted = true;
        }
        if let Some(line) = self.typed_ahead.pop_front() {
            self.prompted = false;
            return Some(line);
        }
        loop {
            let line = self.typed.next_line().await.ok()??;
            if !line.trim().is_empty() {
                self.prompted = false;
                return Some(line);
            }
        }
    }

    /// The order to stop, typed while the Agent works. Anything else typed
    /// meanwhile is not taken as a change of mind, as a terminal is typed
    /// ahead in: it waits for its turn.
    async fn interruption(&mut self) -> Option<String> {
        loop {
            let line = self.typed.next_line().await.ok()??;
            if crate::agent::means_stop(&line) {
                return Some(line);
            }
            if !line.trim().is_empty() {
                self.typed_ahead.push_back(line);
            }
        }
    }

    async fn send(&mut self, text: &str) -> io::Result<()> {
        // Said on its own initiative, it starts on a line of its own.
        let start = if std::mem::take(&mut self.prompted) { "\n" } else { "" };
        self.show(&format!("{start}{text}\n\n")).await
    }

    async fn send_file(&mut self, file: &std::path::Path, caption: &str) -> io::Result<()> {
        let start = if std::mem::take(&mut self.prompted) { "\n" } else { "" };
        self.show(&format!("{start}[file: {}] {caption}\n\n", file.display())).await
    }

    async fn echo(&mut self, text: &str) {
        let start = if std::mem::take(&mut self.prompted) { "\n" } else { "" };
        let _ = self.show(&format!("{start}(you, elsewhere) {text}\n")).await;
    }

    async fn ask(&mut self, approval: &Approval) -> io::Result<Answer> {
        let warning = if approval.irreversible { "This cannot be undone.\n" } else { "" };
        let choices = if approval.critical { "[y/N]" } else { "[y/N/a(lways)]" };
        self.show(&format!(
            "[{}]\n{}\n{warning}Allow? {choices} ",
            approval.summary, approval.detail
        ))
        .await?;
        // What was typed before the question was shown is no answer to it.
        if let Some(ahead) = self.typed_ahead.pop_front() {
            return Ok(Answer::Wrote(ahead));
        }
        let typed = self.typed.next_line().await?.unwrap_or_default();
        Ok(match typed.trim() {
            "y" | "Y" | "yes" => Answer::Allowed,
            "a" | "A" | "always" if !approval.critical => Answer::AlwaysAllowed,
            "" | "n" | "N" | "no" => Answer::Denied,
            other => Answer::Wrote(other.to_owned()),
        })
    }
}
