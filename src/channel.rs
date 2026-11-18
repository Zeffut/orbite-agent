use std::future::Future;
use std::io;

/// A place where the Conversation happens.
pub trait Channel: Send {
    /// The Owner's next message, or `None` once the Channel is closed.
    fn receive(&mut self) -> impl Future<Output = Option<String>> + Send;

    fn send(&mut self, text: &str) -> impl Future<Output = io::Result<()>> + Send;
}
