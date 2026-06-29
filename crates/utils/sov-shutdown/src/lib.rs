//! Shutdown controllers and graceful-shutdown utilities used by full node
//! services.
//!
//! This code is **never** used inside of zkVMs, so it can be non-deterministic,
//! access system resources or networking, write data to disk, etc..

#![deny(missing_docs)]

mod background_task;
mod shutdown_controller;

use std::future::Future;

pub use background_task::BackgroundTask;
pub use shutdown_controller::{
    PrimaryShutdownController, RunnerShutdownController, SecondaryShutdownController,
};
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
/// * `$shutdown` – `&PrimaryShutdownController` for graceful stop
/// * `$var => $body` – code run for each received item
///
/// Logs:
/// * `trace!(%name, "Message received")` for each message
/// * `debug!(%name, "Shutdown signal received, stopping task")` on shutdown
/// * `debug!(%name, "Stream/channel closed, stopping task")` on close
///
/// # Example
/// ```ignore
/// sov_shutdown::consume_until_shutdown!(
///     "WebSocket reader",
///     ws_reader.next(),
///     primary_shutdown,
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
                _ = $shutdown.wait_for_shutdown() => {
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
