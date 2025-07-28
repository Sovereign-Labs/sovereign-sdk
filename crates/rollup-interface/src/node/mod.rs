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
