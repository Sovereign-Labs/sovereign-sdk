//! The primary ("main") shutdown signal for a running rollup.

use std::future::Future;

use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use tokio::sync::watch;

/// Owns both ends of the rollup's *primary* shutdown channel.
///
/// The primary shutdown signal tears down the whole rollup: when it fires, the
/// runner's main loop stops and every background task subscribed to it begins
/// its graceful shutdown. The controller is the single source of truth for this
/// channel — it is created once during startup, lets callers [`trigger`] the
/// shutdown, await it via [`recv_shutdown`], or obtain a raw receiver with
/// [`subscribe_shutdown`] for the lower-level APIs that consume one directly.
///
/// This is distinct from the *secondary* shutdown signal, which is fired only
/// after the runner's main loop has finished, to drain the HTTP/RPC servers and
/// other peripheral tasks in the correct order.
///
/// [`subscribe_shutdown`]: PrimaryShutdownController::subscribe_shutdown
/// [`recv_shutdown`]: PrimaryShutdownController::recv_shutdown
/// [`trigger`]: PrimaryShutdownController::trigger
#[derive(Clone, Debug)]
pub struct PrimaryShutdownController {
    sender: watch::Sender<()>,
    receiver: watch::Receiver<()>,
}

impl PrimaryShutdownController {
    /// Creates a fresh primary shutdown channel.
    ///
    /// The held receiver is marked unchanged so that subscribers do not observe
    /// the channel's initial value as a spurious shutdown.
    pub fn new() -> Self {
        let (sender, mut receiver) = watch::channel(());
        receiver.mark_unchanged();
        Self { sender, receiver }
    }

    /// Returns a fresh receiver for a background task to listen for shutdown on.
    pub fn subscribe_shutdown(&self) -> watch::Receiver<()> {
        self.receiver.clone()
    }

    /// Resolves once a shutdown has been triggered — including if it was
    /// already triggered before this method was called.
    ///
    /// This is the convenient way for a task to await shutdown in a
    /// [`tokio::select!`] arm. For APIs that consume a [`watch::Receiver`]
    /// directly (e.g. synchronous `has_changed` polls or combinators that take
    /// a receiver), use [`subscribe_shutdown`](Self::subscribe_shutdown) instead.
    pub async fn recv_shutdown(&self) {
        // Clone the controller's reference receiver, which is never advanced
        // past the channel's initial version. A clone therefore still observes
        // a shutdown that fired before this call, rather than blocking forever.
        let mut receiver = self.receiver.clone();
        let _ = receiver.changed().await;
    }

    /// Returns `true` if a shutdown has already been triggered.
    ///
    /// This is the synchronous, non-awaiting counterpart to
    /// [`recv_shutdown`](Self::recv_shutdown), for code that needs to branch on
    /// the shutdown state without suspending. It inspects the controller's
    /// reference receiver, which is never advanced past the channel's initial
    /// version, so it reflects whether [`trigger`](Self::trigger) was ever
    /// called.
    pub fn has_changed(&self) -> bool {
        // Conservative default: if the channel is closed (the underlying
        // `has_changed` errors), assume we are shutting down. Callers branch as
        // `if has_changed() { /* stop */ }`, so `true` is the safe fallback.
        self.receiver.has_changed().unwrap_or(true)
    }

    /// Runs `fut` until it completes or a shutdown is triggered, whichever
    /// happens first.
    ///
    /// This is the controller-native form of the free [`future_or_shutdown`]
    /// combinator: it lets a task race a future against shutdown without having
    /// to hold a [`watch::Receiver`] itself. Like [`recv_shutdown`], it observes
    /// a shutdown that fired before the call.
    ///
    /// [`recv_shutdown`]: Self::recv_shutdown
    pub async fn future_or_shutdown<F: Future>(&self, fut: F) -> FutureOrShutdownOutput<F::Output> {
        future_or_shutdown(fut, &self.receiver).await
    }

    /// Triggers the primary shutdown.
    ///
    /// Returns `false` if the signal could not be delivered because every
    /// receiver has already been dropped.
    pub fn trigger(&self) -> bool {
        self.sender.send(()).is_ok()
    }
}

impl Default for PrimaryShutdownController {
    fn default() -> Self {
        Self::new()
    }
}
