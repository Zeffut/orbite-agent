use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, Lines};

use crate::Channel;

/// A Channel for developers: the Conversation typed and read in a terminal.
pub struct Terminal<R, W> {
    typed: Lines<R>,
    shown: W,
}

impl<R: AsyncBufRead + Unpin + Send, W: AsyncWrite + Unpin + Send> Terminal<R, W> {
    pub fn new(typed: R, shown: W) -> Self {
        Self {
            typed: typed.lines(),
            shown,
        }
    }

    async fn show(&mut self, text: &str) -> io::Result<()> {
        self.shown.write_all(text.as_bytes()).await?;
        self.shown.flush().await
    }
}

impl<R: AsyncBufRead + Unpin + Send, W: AsyncWrite + Unpin + Send> Channel for Terminal<R, W> {
    async fn receive(&mut self) -> Option<String> {
        self.show("> ").await.ok()?;
        loop {
            let line = self.typed.next_line().await.ok()??;
            if !line.trim().is_empty() {
                return Some(line);
            }
        }
    }

    async fn send(&mut self, text: &str) -> io::Result<()> {
        self.show(&format!("{text}\n\n")).await
    }
}
