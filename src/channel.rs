use std::future::Future;
use std::io;

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
