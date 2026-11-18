use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, Lines};

use crate::{Approval, Channel};

/// A Channel for developers: the Conversation typed and read in a terminal.
pub struct Terminal<R, W> {
    typed: Lines<R>,
    shown: W,
    /// True while a prompt is on screen with nothing typed after it yet.
    prompted: bool,
}

impl<R: AsyncBufRead + Unpin + Send, W: AsyncWrite + Unpin + Send> Terminal<R, W> {
    pub fn new(typed: R, shown: W) -> Self {
        Self { typed: typed.lines(), shown, prompted: false }
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
        loop {
            let line = self.typed.next_line().await.ok()??;
            if !line.trim().is_empty() {
                self.prompted = false;
                return Some(line);
            }
        }
    }

    async fn send(&mut self, text: &str) -> io::Result<()> {
        // Said on its own initiative, it starts on a line of its own.
        let start = if std::mem::take(&mut self.prompted) { "\n" } else { "" };
        self.show(&format!("{start}{text}\n\n")).await
    }

    async fn ask(&mut self, approval: &Approval) -> io::Result<bool> {
        let warning = if approval.irreversible { "This cannot be undone.\n" } else { "" };
        self.show(&format!("[{}]\n{}\n{warning}Allow? [y/N] ", approval.summary, approval.detail))
            .await?;
        let answer = self.typed.next_line().await?.unwrap_or_default();
        Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
    }
}
