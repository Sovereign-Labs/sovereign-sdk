//! The primary ("main") shutdown signal for a running rollup.

use tokio::sync::watch;

/// Owns both ends of the rollup's *primary* shutdown channel.
///
/// The primary shutdown signal tears down the whole rollup: when it fires, the
/// runner's main loop stops and every background task subscribed to it begins
/// its graceful shutdown. The controller is the single source of truth for this
/// channel — it is created once during startup, lets callers [`trigger`] the
/// shutdown, await it via [`recv_shutdown`], or obtain a raw receiver with
/// [`subscribe`] for the lower-level APIs that consume one directly.
///
/// This is distinct from the *secondary* shutdown signal, which is fired only
/// after the runner's main loop has finished, to drain the HTTP/RPC servers and
/// other peripheral tasks in the correct order.
///
/// [`subscribe`]: PrimaryShutdownController::subscribe
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
    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.receiver.clone()
    }

    /// Resolves once a shutdown has been triggered — including if it was
    /// already triggered before this method was called.
    ///
    /// This is the convenient way for a task to await shutdown in a
    /// [`tokio::select!`] arm. For APIs that consume a [`watch::Receiver`]
    /// directly (e.g. synchronous `has_changed` polls or combinators that take
    /// a receiver), use [`subscribe`](Self::subscribe) instead.
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
        // The controller always holds a receiver, so the channel is never
        // closed and `has_changed` cannot error here.
        self.receiver.has_changed().unwrap_or(false)
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
