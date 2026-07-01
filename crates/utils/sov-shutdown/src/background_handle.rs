use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::task::{JoinError, JoinHandle};

tokio::task_local! {
    /// Name of the [`BackgroundHandle`] task currently executing on this Tokio
    /// task. Set for the lifetime of the spawned future so the global panic
    /// hook can recover *which* background task panicked.
    static TASK_NAME: &'static str;
}

/// Returns the name of the [`BackgroundHandle`] task running on the current
/// Tokio task, or `None` if the current task was not spawned via
/// [`BackgroundHandle::spawn`] (e.g. a raw `tokio::spawn`).
pub(crate) fn current_task_name() -> Option<&'static str> {
    TASK_NAME.try_with(|name| *name).ok()
}

/// A handle to a spawned background task that participates in graceful
/// shutdown.
///
/// Thin wrapper over a [`tokio::task::JoinHandle`] used to track long-running
/// background work (HTTP servers, block fetchers, etc.) so it can be awaited to
/// completion once a shutdown signal has been sent. Awaiting a [`BackgroundHandle`]
/// behaves exactly like awaiting the underlying [`JoinHandle`], so it is a
/// drop-in replacement wherever a join handle was previously stored.
pub struct BackgroundHandle<T> {
    handle: JoinHandle<T>,
}

impl<T> BackgroundHandle<T> {
    /// Spawns `future` on the current Tokio runtime and tracks it as a
    /// background task under the human-readable `name`. The name is recorded in
    /// a task-local for the lifetime of the task so the global panic hook can
    /// report which task panicked (see [`current_task_name`]).
    pub fn spawn<F>(name: &'static str, future: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        Self {
            handle: tokio::spawn(TASK_NAME.scope(name, future)),
        }
    }

    /// Forces the background task to be cancelled.
    ///
    /// See [`tokio::task::JoinHandle::abort`] for the exact semantics.
    pub fn abort(&self) {
        self.handle.abort();
    }

    /// Returns `true` if the background task has finished.
    ///
    /// See [`tokio::task::JoinHandle::is_finished`] for the exact semantics.
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

/// Awaiting a [`BackgroundHandle`] resolves to the task's output, or a
/// [`JoinError`] if the task panicked or was aborted — identical to awaiting the
/// underlying [`JoinHandle`].
impl<T> Future for BackgroundHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.handle).poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use sov_test_utils::logging::LogCollector;
    use tracing::Level;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry;

    use super::*;
    use crate::set_tracing_panic_hook;

    #[tokio::test(flavor = "multi_thread")]
    async fn panicking_task_is_logged_with_name_and_message() {
        // `Level::INFO` captures both the healthy task's info log and the panic
        // hook's error log. The collector is installed globally because the panic
        // hook runs on the panicking task's worker thread.
        let collector = LogCollector::new(Level::INFO);
        registry().with(collector.clone()).init();
        set_tracing_panic_hook();

        let healthy = BackgroundHandle::spawn("healthy-task", async {
            tracing::info!("healthy task {} running", current_task_name().unwrap());
        });

        let panicker = BackgroundHandle::spawn("panicking-task", async {
            panic!("boom: deliberate test panic");
        });

        // Awaiting the panicker yields the caught panic; we only care about logs.
        let panic_result = panicker.await;
        assert!(
            panic_result.is_err_and(|e| e.is_panic()),
            "the panicking task should surface a panic on join"
        );

        let logged = |needle: &str| collector.records().iter().any(|(_, m)| m.contains(needle));

        assert!(
            logged("panic in background task panicking-task: boom: deliberate test panic"),
            "the panic hook should log the panicking task's name and panic message; got: {:?}",
            collector.records()
        );

        healthy.await.unwrap();

        assert!(
            logged("healthy-task"),
            "healthy task's name should appear in its own log; got: {:?}",
            collector.records()
        );
    }
}
