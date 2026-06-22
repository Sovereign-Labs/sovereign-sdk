//! The primary ("main") shutdown signal for a running rollup.

use tokio::sync::watch;

/// Owns both ends of the rollup's *primary* shutdown channel.
///
/// The primary shutdown signal tears down the whole rollup: when it fires, the
/// runner's main loop stops and every background task subscribed to it begins
/// its graceful shutdown. The controller is the single source of truth for this
/// channel — it is created once during startup, hands out [`subscribe`] receivers
/// to background tasks, hands out [`sender`] clones to components that may need to
/// initiate shutdown themselves (e.g. fatal-error paths), and exposes [`trigger`]
/// for the orchestration layer to start the shutdown.
///
/// This is distinct from the *secondary* shutdown signal, which is fired only
/// after the runner's main loop has finished, to drain the HTTP/RPC servers and
/// other peripheral tasks in the correct order.
///
/// [`subscribe`]: PrimaryShutdownController::subscribe
/// [`sender`]: PrimaryShutdownController::sender
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
