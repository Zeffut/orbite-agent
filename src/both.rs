use std::io;
use std::path::Path;

use crate::{Answer, Approval, Channel, Step};

/// The one Conversation open in two places at once. What the Owner writes in
/// either is shown in the other, the Agent's words reach both, and an
/// Approval asked in both is answered once, from either. More places are
/// had by putting a `Both` inside another.
pub struct Both<A, B> {
    first: A,
    second: B,
    /// False once that place has closed; the Conversation goes on in the other.
    first_open: bool,
    second_open: bool,
}

impl<A: Channel, B: Channel> Both<A, B> {
    pub fn new(first: A, second: B) -> Self {
        Self { first, second, first_open: true, second_open: true }
    }
}

/// Which of the two places something came from.
#[derive(Clone, Copy)]
enum Place {
    First,
    Second,
}

impl<A: Channel, B: Channel> Both<A, B> {
    /// Shows in the other place what the Owner wrote in `place`.
    async fn echo_from(&mut self, place: Place, text: &str) {
        match place {
            Place::First if self.second_open => self.second.echo(text).await,
            Place::Second if self.first_open => self.first.echo(text).await,
            _ => {}
        }
    }

    fn close(&mut self, place: Place) {
        match place {
            Place::First => self.first_open = false,
            Place::Second => self.second_open = false,
        }
    }
}

impl<A: Channel, B: Channel> Channel for Both<A, B> {
    async fn receive(&mut self) -> Option<String> {
        loop {
            let (place, written) = tokio::select! {
                written = self.first.receive(), if self.first_open => (Place::First, written),
                written = self.second.receive(), if self.second_open => (Place::Second, written),
                else => return None,
            };
            match written {
                Some(text) => {
                    self.echo_from(place, &text).await;
                    return Some(text);
                }
                None => self.close(place),
            }
        }
    }

    async fn written_before(&mut self) -> Vec<String> {
        let mut written = self.first.written_before().await;
        written.extend(self.second.written_before().await);
        written
    }

    async fn erase_last_received(&mut self) {
        self.first.erase_last_received().await;
        self.second.erase_last_received().await;
    }

    async fn erase_received_holding(&mut self, text: &str) {
        self.first.erase_received_holding(text).await;
        self.second.erase_received_holding(text).await;
    }

    async fn interruption(&mut self) -> Option<String> {
        loop {
            let (place, written) = tokio::select! {
                written = self.first.interruption(), if self.first_open => (Place::First, written),
                written = self.second.interruption(), if self.second_open => (Place::Second, written),
                else => return None,
            };
            match written {
                Some(text) => {
                    self.echo_from(place, &text).await;
                    return Some(text);
                }
                None => self.close(place),
            }
        }
    }

    async fn send_unkept(&mut self, text: &str) -> io::Result<()> {
        if self.first_open && self.first.send_unkept(text).await.is_err() {
            self.first_open = false;
        }
        if self.second_open && self.second.send_unkept(text).await.is_err() {
            self.second_open = false;
        }
        if self.first_open || self.second_open {
            Ok(())
        } else {
            Err(io::Error::other("every place the Conversation was open in has closed"))
        }
    }

    async fn send(&mut self, text: &str) -> io::Result<()> {
        if self.first_open && self.first.send(text).await.is_err() {
            self.first_open = false;
        }
        if self.second_open && self.second.send(text).await.is_err() {
            self.second_open = false;
        }
        if self.first_open || self.second_open {
            Ok(())
        } else {
            Err(io::Error::other("every place the Conversation was open in has closed"))
        }
    }

    async fn send_file(&mut self, file: &Path, caption: &str) -> io::Result<()> {
        let first =
            if self.first_open { self.first.send_file(file, caption).await } else { Ok(()) };
        let second =
            if self.second_open { self.second.send_file(file, caption).await } else { Ok(()) };
        // Reaching the Owner in one place is reaching them.
        first.or(second)
    }

    async fn ask(&mut self, approval: &Approval) -> io::Result<Answer> {
        loop {
            // Asked in both; whichever is answered first decides.
            let (place, answer) = tokio::select! {
                answer = self.first.ask(approval), if self.first_open => (Place::First, answer),
                answer = self.second.ask(approval), if self.second_open => (Place::Second, answer),
                else => return Err(io::Error::other("every place the Conversation was open in has closed")),
            };
            let Ok(answer) = answer else {
                self.close(place);
                continue;
            };
            match place {
                Place::First if self.second_open => self.second.settle(approval, &answer).await,
                Place::Second if self.first_open => self.first.settle(approval, &answer).await,
                _ => {}
            }
            if let Answer::Wrote(text) = &answer {
                self.echo_from(place, text).await;
            }
            return Ok(answer);
        }
    }

    async fn offer(&mut self, text: &str, choices: &[(String, String)]) -> io::Result<()> {
        if self.first_open && self.first.offer(text, choices).await.is_err() {
            self.close(Place::First);
        }
        if self.second_open && self.second.offer(text, choices).await.is_err() {
            self.close(Place::Second);
        }
        if self.first_open || self.second_open {
            Ok(())
        } else {
            Err(io::Error::other("every place the Conversation was open in has closed"))
        }
    }

    async fn echo(&mut self, text: &str) {
        if self.first_open {
            self.first.echo(text).await;
        }
        if self.second_open {
            self.second.echo(text).await;
        }
    }

    async fn settle(&mut self, approval: &Approval, answer: &Answer) {
        if self.first_open {
            self.first.settle(approval, answer).await;
        }
        if self.second_open {
            self.second.settle(approval, answer).await;
        }
    }

    async fn screen(&mut self, picture: &Path) {
        if self.first_open {
            self.first.screen(picture).await;
        }
        if self.second_open {
            self.second.screen(picture).await;
        }
    }

    async fn doing(&mut self, step: &Step) {
        if self.first_open {
            self.first.doing(step).await;
        }
        if self.second_open {
            self.second.doing(step).await;
        }
    }

    async fn idle(&mut self) {
        if self.first_open {
            self.first.idle().await;
        }
        if self.second_open {
            self.second.idle().await;
        }
    }

    async fn working(&mut self) {
        if self.first_open {
            self.first.working().await;
        }
        if self.second_open {
            self.second.working().await;
        }
    }
}
