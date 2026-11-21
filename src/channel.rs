use std::future::Future;
use std::io;
use std::path::Path;

use crate::Step;

/// What the Owner is shown when the Agent asks permission before an Action.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Approval {
    /// The Action in a few words, such as "Run a command".
    pub summary: String,
    /// Exactly what would be done.
    pub detail: String,
    /// True when the Action cannot be undone once carried out.
    pub irreversible: bool,
    /// True for a Critical Action: it is asked about at every Permission
    /// Level and cannot be allowed once and for all.
    pub critical: bool,
    /// True when the Action only touches the Agent's own folders.
    pub contained: bool,
}

/// How the Owner answered an [`Approval`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Allowed,
    /// Allowed, and not to be asked about again.
    AlwaysAllowed,
    Denied,
    /// The Owner wrote this instead of answering: the Action is not allowed,
    /// and what they wrote is to be heard.
    Wrote(String),
}

/// A place where the Conversation happens.
pub trait Channel: Send {
    /// The Owner's next message, or `None` once the Channel is closed.
    fn receive(&mut self) -> impl Future<Output = Option<String>> + Send;

    fn send(&mut self, text: &str) -> impl Future<Output = io::Result<()>> + Send;

    /// Asks the Owner to allow or deny an Action, and waits for the answer.
    /// A Channel with no way to ask denies.
    fn ask(&mut self, approval: &Approval) -> impl Future<Output = io::Result<Answer>> + Send {
        let _ = approval;
        async { Ok(Answer::Denied) }
    }

    /// What the Owner writes while the Agent is working, so that they can
    /// stop or redirect it. Must be safe to abandon half-way. A Channel that
    /// cannot listen while the Agent works never yields anything.
    fn interruption(&mut self) -> impl Future<Output = Option<String>> + Send {
        std::future::pending()
    }

    /// Sends the Owner a file, with a few words. A Channel that cannot
    /// carry files says so.
    fn send_file(
        &mut self,
        file: &Path,
        caption: &str,
    ) -> impl Future<Output = io::Result<()>> + Send {
        let _ = (file, caption);
        async { Err(io::Error::other("this Channel cannot carry files")) }
    }

    /// Shows the Owner something they wrote in another place where the
    /// Conversation is open, so that every place holds all of it.
    fn echo(&mut self, text: &str) -> impl Future<Output = ()> + Send {
        let _ = text;
        async {}
    }

    /// Called when an Approval asked here was answered in another place, so
    /// that the question shown here closes with the decision.
    fn settle(&mut self, approval: &Approval, answer: &Answer) -> impl Future<Output = ()> + Send {
        let _ = (approval, answer);
        async {}
    }

    /// Removes from where the Conversation is shown what the Owner last
    /// wrote, when it should not stay there. Does nothing where that is not
    /// possible.
    fn erase_last_received(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Removes from where the Conversation is shown what the Owner wrote
    /// lately that holds `text`: a secret they wrote in passing. Does
    /// nothing where that is not possible.
    fn erase_received_holding(&mut self, text: &str) -> impl Future<Output = ()> + Send {
        let _ = text;
        async {}
    }

    /// Sends the Owner a text with choices under it, each a few words and
    /// the command that choosing it stands for: choosing one is received as
    /// if the Owner had written that command. A Channel with no buttons
    /// writes the commands out.
    fn offer(
        &mut self,
        text: &str,
        choices: &[(String, String)],
    ) -> impl Future<Output = io::Result<()>> + Send {
        let mut written = text.to_owned();
        for (label, command) in choices {
            written.push_str(&format!("\n{label}: {command}"));
        }
        async move { self.send(&written).await }
    }

    /// Shows the Owner what the Agent is starting to do.
    fn doing(&mut self, step: &Step) -> impl Future<Output = ()> + Send {
        let _ = step;
        async {}
    }

    /// Called when the Agent has nothing under way any more, so that nothing
    /// shown by [`Channel::doing`] is left behind.
    fn idle(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Shows the Owner that the Agent is busy. Called again every few seconds
    /// for as long as it is; a Channel with no way to show it does nothing.
    fn working(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }
}
