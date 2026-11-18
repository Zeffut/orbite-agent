use std::future::Future;
use std::io;

/// A place where the Conversation happens.
pub trait Channel: Send {
    /// The Owner's next message, or `None` once the Channel is closed.
    fn receive(&mut self) -> impl Future<Output = Option<String>> + Send;

    fn send(&mut self, text: &str) -> impl Future<Output = io::Result<()>> + Send;

    /// Shows the Owner that the Agent is busy. Called again every few seconds
    /// for as long as it is; a Channel with no way to show it does nothing.
    fn working(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }
}
