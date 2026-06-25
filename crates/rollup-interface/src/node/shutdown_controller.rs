use std::future::Future;

use tokio::sync::watch;

use super::{future_or_shutdown, FutureOrShutdownOutput};

#[derive(Clone)]
struct InnerShutdownController {
    sender: watch::Sender<()>,
    receiver: watch::Receiver<()>,
}

impl InnerShutdownController {
    fn new() -> Self {
        let (sender, mut receiver) = watch::channel(());
        receiver.mark_unchanged();
        Self { sender, receiver }
    }

    async fn wait_for_shutdown(&self) -> Result<(), watch::error::RecvError> {
        let mut shutdown_receiver = self.receiver.clone();
        shutdown_receiver.changed().await
    }

    fn shutdown(&self) {
        // The controller keeps its own receiver alive for as long as it exists,
        // so there is always at least one receiver and `send` cannot fail here.
        self.sender
            .send(())
            .expect("shutdown channel always has a live receiver");
    }

    fn is_triggered(&self) -> bool {
        // The controller keeps its own sender alive for as long as it exists,
        // so `has_changed` cannot fail here.
        self.receiver
            .has_changed()
            .expect("shutdown channel always has a live sender")
    }
}

/// Controls the primary shutdown channel used to stop the main runner.
#[derive(Clone)]
pub struct PrimaryShutdownController {
    inner: InnerShutdownController,
}

impl PrimaryShutdownController {
    /// Creates a new primary shutdown controller.
    pub fn new() -> Self {
        Self {
            inner: InnerShutdownController::new(),
        }
    }

    /// Waits until a primary shutdown notification is sent.
    pub async fn wait_for_shutdown(&self) -> Result<(), watch::error::RecvError> {
        self.inner.wait_for_shutdown().await
    }

    /// Runs a future until it completes or the primary shutdown signal fires.
    pub async fn future_or_shutdown<T>(&self, inner: T) -> FutureOrShutdownOutput<T::Output>
    where
        T: Future,
    {
        future_or_shutdown(inner, &self.inner.receiver).await
    }

    /// Sends a primary shutdown notification.
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }

    /// Returns `true` if a primary shutdown notification has already been sent.
    pub fn is_triggered(&self) -> bool {
        self.inner.is_triggered()
    }
}

impl Default for PrimaryShutdownController {
    fn default() -> Self {
        Self::new()
    }
}

/// Controls the secondary shutdown channel used after the main runner exits.
#[derive(Clone)]
pub struct SecondaryShutdownController {
    inner: InnerShutdownController,
}

impl SecondaryShutdownController {
    /// Creates a new secondary shutdown controller.
    pub fn new() -> Self {
        Self {
            inner: InnerShutdownController::new(),
        }
    }

    /// Waits until a secondary shutdown notification is sent.
    pub async fn wait_for_shutdown(&self) -> Result<(), watch::error::RecvError> {
        self.inner.wait_for_shutdown().await
    }

    /// Runs a future until it completes or the secondary shutdown signal fires.
    pub async fn future_or_shutdown<T>(&self, inner: T) -> FutureOrShutdownOutput<T::Output>
    where
        T: Future,
    {
        future_or_shutdown(inner, &self.inner.receiver).await
    }

    /// Sends a secondary shutdown notification.
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
}

impl Default for SecondaryShutdownController {
    fn default() -> Self {
        Self::new()
    }
}
