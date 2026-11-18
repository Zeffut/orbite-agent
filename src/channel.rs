use std::future::Future;
use std::io;

/// What the Owner is shown when the Agent asks permission before an Action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    /// The Action in a few words, such as "Run a command".
    pub summary: String,
    /// Exactly what would be done.
    pub detail: String,
    /// True when the Action cannot be undone once carried out.
    pub irreversible: bool,
}

/// A place where the Conversation happens.
pub trait Channel: Send {
    /// The Owner's next message, or `None` once the Channel is closed.
    fn receive(&mut self) -> impl Future<Output = Option<String>> + Send;

    fn send(&mut self, text: &str) -> impl Future<Output = io::Result<()>> + Send;

    /// Asks the Owner to allow or deny an Action, and waits for the answer.
    /// A Channel with no way to ask denies.
    fn ask(&mut self, approval: &Approval) -> impl Future<Output = io::Result<bool>> + Send {
        let _ = approval;
        async { Ok(false) }
    }

    /// Shows the Owner that the Agent is busy. Called again every few seconds
    /// for as long as it is; a Channel with no way to show it does nothing.
    fn working(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }
}
