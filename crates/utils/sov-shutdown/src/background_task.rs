use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::task::{JoinError, JoinHandle};

/// A handle to a spawned background task that participates in graceful
/// shutdown.
///
/// Thin wrapper over a [`tokio::task::JoinHandle`] used to track long-running
/// background work (HTTP servers, block fetchers, etc.) so it can be awaited to
/// completion once a shutdown signal has been sent. Awaiting a [`BackgroundTask`]
/// behaves exactly like awaiting the underlying [`JoinHandle`], so it is a
/// drop-in replacement wherever a join handle was previously stored.
pub struct BackgroundTask<T> {
    handle: JoinHandle<T>,
}

impl<T> BackgroundTask<T> {
    /// Wraps an existing [`JoinHandle`] as a tracked background task.
    pub fn new(handle: JoinHandle<T>) -> Self {
        Self { handle }
    }

    /// Spawns `future` on the current Tokio runtime and tracks it as a
    /// background task.
    pub fn spawn<F>(future: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        Self::new(tokio::spawn(future))
    }

    /// Forces the background task to be cancelled.
    ///
    /// See [`tokio::task::JoinHandle::abort`] for the exact semantics.
    pub fn abort(&self) {
        self.handle.abort();
    }
}

impl<T> From<JoinHandle<T>> for BackgroundTask<T> {
    fn from(handle: JoinHandle<T>) -> Self {
        Self::new(handle)
    }
}

/// Awaiting a [`BackgroundTask`] resolves to the task's output, or a
/// [`JoinError`] if the task panicked or was aborted — identical to awaiting the
/// underlying [`JoinHandle`].
impl<T> Future for BackgroundTask<T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.handle).poll(cx)
    }
}
