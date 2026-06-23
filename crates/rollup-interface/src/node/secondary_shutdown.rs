use std::future::Future;

use tokio::sync::watch;

use super::{future_or_shutdown, FutureOrShutdownOutput};

/// Controls the secondary shutdown channel used after the main runner exits.
#[derive(Clone)]
pub struct SecondaryShutdownController {
    sender: watch::Sender<()>,
    receiver: watch::Receiver<()>,
}

impl SecondaryShutdownController {
    /// Creates a new secondary shutdown controller.
    pub fn new() -> Self {
        let (sender, mut receiver) = watch::channel(());
        receiver.mark_unchanged();
        Self { sender, receiver }
    }

    /// Waits until a secondary shutdown notification is sent.
    pub async fn changed(&self) -> Result<(), watch::error::RecvError> {
        let mut shutdown_receiver = self.receiver.clone();
        shutdown_receiver.changed().await
    }

    /// Runs a future until it completes or the secondary shutdown signal fires.
    pub async fn future_or_shutdown_secondary<T>(
        &self,
        inner: T,
    ) -> FutureOrShutdownOutput<T::Output>
    where
        T: Future,
    {
        future_or_shutdown(inner, &self.receiver).await
    }

    /// Sends a secondary shutdown notification.
    pub fn shutdown(&self) -> Result<(), watch::error::SendError<()>> {
        self.sender.send(())
    }
}

impl Default for SecondaryShutdownController {
    fn default() -> Self {
        Self::new()
    }
}
