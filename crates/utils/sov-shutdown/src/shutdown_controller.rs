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

    fn shutdown_with_location(&self, location: &'static std::panic::Location<'static>) {
        tracing::info!(
            file = location.file(),
            line = location.line(),
            column = location.column(),
            "Shutdown triggered",
        );
        self.shutdown();
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

    /// Sends a primary shutdown notification, logging the call site that
    /// triggered it.
    #[track_caller]
    pub fn shutdown(&self) {
        self.inner
            .shutdown_with_location(std::panic::Location::caller());
    }

    /// Sends a primary shutdown notification, logging the supplied `location`.
    /// Use this when the triggering call site was captured earlier (e.g. across
    /// an async boundary); otherwise prefer [`Self::shutdown`].
    pub fn shutdown_with_location(&self, location: &'static std::panic::Location<'static>) {
        self.inner.shutdown_with_location(location);
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

    /// Sends a secondary shutdown notification, logging the call site that
    /// triggered it.
    #[track_caller]
    pub fn shutdown(&self) {
        self.inner
            .shutdown_with_location(std::panic::Location::caller());
    }

    /// Sends a secondary shutdown notification, logging the supplied `location`.
    /// Use this when the triggering call site was captured earlier (e.g. across
    /// an async boundary); otherwise prefer [`Self::shutdown`].
    pub fn shutdown_with_location(&self, location: &'static std::panic::Location<'static>) {
        self.inner.shutdown_with_location(location);
    }
}

impl Default for SecondaryShutdownController {
    fn default() -> Self {
        Self::new()
    }
}

/// Controls the shutdown channel for the runner's own background tasks
/// (HTTP server, sync-status updater, finalized-block fetcher).
#[derive(Clone)]
pub struct RunnerShutdownController {
    inner: InnerShutdownController,
}

impl RunnerShutdownController {
    /// Creates a new runner shutdown controller.
    pub fn new() -> Self {
        Self {
            inner: InnerShutdownController::new(),
        }
    }

    /// Waits until a runner shutdown notification is sent.
    pub async fn wait_for_shutdown(&self) -> Result<(), watch::error::RecvError> {
        self.inner.wait_for_shutdown().await
    }

    /// Runs a future until it completes or the runner shutdown signal fires.
    pub async fn future_or_shutdown<T>(&self, inner: T) -> FutureOrShutdownOutput<T::Output>
    where
        T: Future,
    {
        future_or_shutdown(inner, &self.inner.receiver).await
    }

    /// Sends a runner shutdown notification, logging the call site that
    /// triggered it.
    #[track_caller]
    pub fn shutdown(&self) {
        self.inner
            .shutdown_with_location(std::panic::Location::caller());
    }

    /// Sends a runner shutdown notification, logging the supplied `location`.
    /// Use this when the triggering call site was captured earlier (e.g. across
    /// an async boundary); otherwise prefer [`Self::shutdown`].
    pub fn shutdown_with_location(&self, location: &'static std::panic::Location<'static>) {
        self.inner.shutdown_with_location(location);
    }
}

impl Default for RunnerShutdownController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use tracing_test::traced_test;

    use super::*;

    #[traced_test]
    #[test]
    fn shutdown_logs_the_caller_location() {
        let (file, line) = test_shutdown();

        assert!(
            logs_contain(&format!("Shutdown triggered file=\"{file}\" line={line}")),
            "shutdown should log the triggered message with the caller's location",
        );
    }

    /// Triggers a shutdown and returns the file and line of the `shutdown()`
    /// call site that the log should be attributed to.
    fn test_shutdown() -> (&'static str, u32) {
        // `shutdown` is `#[track_caller]`, so the logged location should point
        // at this call site (i.e. this source file and line).
        PrimaryShutdownController::new().shutdown();
        (file!(), line!() - 1)
    }
}
