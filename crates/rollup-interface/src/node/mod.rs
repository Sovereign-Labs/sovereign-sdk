//! Types, traits, or utilities that are used by the full node but are not part
//! of the rollup's state machine.
//!
//! This code is **never** used inside of zkVMs, so it can be non-deterministic,
//! access system resources or networking, write data to disk, etc..

pub mod da;
mod da_sync_state;
pub mod ledger_api;

use std::future::Future;

pub use da_sync_state::{DaSyncState, SyncStatus};
use tokio::select;
use tokio::sync::watch;

/// A [`Future`] that can be "interrupted" by a shutdown signal.
pub async fn future_or_shutdown<T>(
    inner: T,
    shutdown: &watch::Receiver<()>,
) -> FutureOrShutdownOutput<T::Output>
where
    T: Future,
{
    let mut shutdown = shutdown.clone();

    select! {
        res = inner => FutureOrShutdownOutput::Output(res),
        _ = shutdown.changed() => FutureOrShutdownOutput::Shutdown,
    }
}

/// The [`Future::Output`] of [`future_or_shutdown`].
pub enum FutureOrShutdownOutput<O> {
    /// The inner future has produced a new value.
    Output(O),
    /// The future should not be polled anymore.
    Shutdown,
}

/// Repeatedly receives messages until shutdown or channel close.
///
/// Works with any async source returning `Option<T>` (e.g. `recv()` or `next()`).
///
/// # Args
/// * `$name` – task name (for structured logs, e.g. `"writer"` or `format!("task-{id}")`)
/// * `$recv` – async expression like `rx.recv()` or `stream.next()`
/// * `$shutdown` – `watch::Receiver<()>` for graceful stop
/// * `$var => $body` – code run for each received item
///
/// Logs:
/// * `trace!(%name, "Message received")` for each message
/// * `debug!(%name, "Shutdown signal received, stopping task")` on shutdown
/// * `debug!(%name, "Stream/channel closed, stopping task")` on close
///
/// # Example
/// ```rust
/// consume_until_shutdown!(
///     "WebSocket reader",
///     ws_reader.next(),
///     shutdown_rx,
///     msg => {
///         handle(m).await,
///     }
/// );
/// ```
#[macro_export]
macro_rules! consume_until_shutdown {
    (
        $name:expr,
        $recv:expr,
        $shutdown:expr,
        $var:ident => $body:block
    ) => {
        let name = $name;
        loop {
            tokio::select! {
                _ = $shutdown.changed() => {
                    tracing::debug!(%name, "Shutdown signal received, stopping task");
                    break;
                }
                maybe_item = $recv => {
                    let Some($var) = maybe_item else {
                        tracing::debug!(%name, "Stream/channel closed, stopping task");
                        break;
                    };
                    tracing::trace!(%name, "Message received");
                    $body
                }
            }
        }
    };
}
