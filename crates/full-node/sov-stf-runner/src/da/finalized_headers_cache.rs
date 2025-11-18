//! Provides a caching layer for DA finalized headers to reduce network calls.
//!
//! This module implements a caching wrapper around [`DaService`] that significantly
//! reduces the number of network calls needed when querying finalized headers.
//!
//! # Architecture
//!
//! The caching is implemented through two main components:
//!
//! 1. **Last Finalized Header Cache**: A background task polls the DA service
//!    at regular intervals and broadcasts updates via a `watch` channel. This allows
//!    multiple consumers to get the latest finalized header without any network call.
//!
//! 2. **Recent Headers Cache**: A bounded LRU-style cache (using `BTreeMap`) stores
//!    the most recent finalized headers. This handles common queries for recently
//!    finalized blocks without hitting the network.
//!
//! # Performance Benefits
//!
//! - Eliminates redundant `get_last_finalized_block_header()` calls during state transitions
//! - Caches up to 30 recent headers to serve `get_block_header_at()` requests
//! - Background polling ensures data freshness without blocking the critical path
//!
//! # Usage
//!
//! ```ignore
//! let da_service_with_cache = DaServiceWithCachedFinalizedHeaders::new(
//!     da_service,
//!     shutdown_receiver,
//!     Duration::from_millis(500), // polling interval
//! ).await?;
//!
//! // Get last finalized header without network call
//! let header = da_service_with_cache.get_last_finalized_block_header()?;
//!
//! // Try to get from cache, fallback to network
//! let header_at_height = da_service_with_cache.get_block_header_at(100).await?;
//! ```

use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Maximum number of recent headers to cache.
///
/// This value balances memory usage against cache hit rate. Headers are evicted
/// in FIFO order when the cache exceeds this size.
const MAX_RECENT_HEADERS: usize = 30;

const RECENT_HEADERS_POISONED: &str = "Recent headers lock is poisoned";

/// Wrapper around [`DaService`] that optimizes interaction with the DA layer.
///
/// This wrapper provides two main optimizations:
/// - **Cached last finalized header**: Accessible without network calls via a background polling task
/// - **Recent headers cache**: Stores up to [`MAX_RECENT_HEADERS`] recent headers to reduce network calls
///
/// The background task continues running until either:
/// - A shutdown signal is received via the `shutdown_receiver`
/// - The DA service returns an error (which is logged and stops the task)
/// - All receivers of the finalized header are dropped
///
/// # Thread Safety
///
/// This struct is `Clone` and can be shared across threads.
/// All internal state is protected by appropriate synchronization primitives (`Arc`, `RwLock`, `watch`).
#[derive(Debug, Clone)]
pub struct DaServiceWithCachedFinalizedHeaders<Da: DaService> {
    // TODO: follow up Remove Arc, DaService already clone
    da_service: Arc<Da>,
    /// Receiver for the last finalized header, updated by background task
    last_finalized: tokio::sync::watch::Receiver<<Da::Spec as DaSpec>::BlockHeader>,
    /// Cache of recently finalized headers, keyed by height
    recent_headers: Arc<std::sync::RwLock<BTreeMap<u64, <Da::Spec as DaSpec>::BlockHeader>>>,
    /// Handle to the background polling task for monitoring its status
    finalized_headers_task: Arc<tokio::task::JoinHandle<()>>,
}

impl<Da: DaService> DaServiceWithCachedFinalizedHeaders<Da> {
    #[allow(missing_docs)]
    pub async fn new(
        da_service: Arc<Da>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
        polling_interval: std::time::Duration,
    ) -> anyhow::Result<Self> {
        let last_finalized_header = da_service
            .get_last_finalized_block_header()
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        let (finalized_sender, finalized_receiver) =
            tokio::sync::watch::channel(last_finalized_header);

        let recent_headers = Arc::new(std::sync::RwLock::new(BTreeMap::new()));

        let recent_headers_for_writer = recent_headers.clone();
        let da_service_for_finalized_fetcher = da_service.clone();

        let finalized_header_handler = tokio::task::spawn(async move {
            background_header_fetch_task(
                da_service_for_finalized_fetcher,
                finalized_sender,
                recent_headers_for_writer,
                polling_interval,
                shutdown_receiver,
            )
            .await;
        });

        Ok(Self {
            da_service,
            last_finalized: finalized_receiver,
            recent_headers,
            finalized_headers_task: Arc::new(finalized_header_handler),
        })
    }
}

// Usage impl
impl<Da: DaService> DaServiceWithCachedFinalizedHeaders<Da> {
    /// Returns the current cached last finalized block header.
    ///
    /// # Errors
    ///
    /// Returns an error if the background polling task has stopped.
    pub fn get_last_finalized_block_header(
        &self,
    ) -> anyhow::Result<<Da::Spec as DaSpec>::BlockHeader> {
        if self.finalized_headers_task.is_finished() {
            anyhow::bail!("DA header provider background task has stopped");
        }
        let header = { self.last_finalized.borrow().clone() };
        Ok(header)
    }

    /// Fetches a block header at the specified height from the DA service.
    ///
    /// This is a direct passthrough to the underlying DA service.
    #[tracing::instrument(skip(self))]
    pub async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Da::Spec as DaSpec>::BlockHeader, Da::Error> {
        let cached_header = {
            let cache = self.recent_headers.read().expect(RECENT_HEADERS_POISONED);
            cache.get(&height).cloned()
        };
        if let Some(cached_header) = cached_header {
            return Ok(cached_header);
        }
        self.da_service.get_block_header_at(height).await
    }

    /// Currently does direct call to underlying [`DaService`], but can be optimized in the future too.
    pub async fn get_head_block_header(
        &self,
    ) -> Result<<Da::Spec as DaSpec>::BlockHeader, Da::Error> {
        self.da_service.get_head_block_header().await
    }
}

// TODO: Switch to subscription when it is brought back
async fn background_header_fetch_task<Da: DaService>(
    da_service: Arc<Da>,
    finalized_sender: tokio::sync::watch::Sender<<Da::Spec as DaSpec>::BlockHeader>,
    recent_headers: Arc<std::sync::RwLock<BTreeMap<u64, <Da::Spec as DaSpec>::BlockHeader>>>,
    polling_interval: std::time::Duration,
    shutdown_rx: tokio::sync::watch::Receiver<()>,
) {
    let mut interval = tokio::time::interval(polling_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Wait for the next tick or shutdown
        match future_or_shutdown(interval.tick(), &shutdown_rx).await {
            FutureOrShutdownOutput::Shutdown => {
                tracing::info!("DA header provider received shutdown signal");
                break;
            }
            FutureOrShutdownOutput::Output(_) => {
                // Fetch finalized header
                match future_or_shutdown(da_service.get_last_finalized_block_header(), &shutdown_rx)
                    .await
                {
                    FutureOrShutdownOutput::Shutdown => {
                        tracing::info!("DA header provider received shutdown signal");
                        break;
                    }
                    FutureOrShutdownOutput::Output(Ok(finalized_header)) => {
                        let height = finalized_header.height();
                        if finalized_sender.send(finalized_header.clone()).is_err() {
                            tracing::debug!(
                                "All DA header provider receivers dropped, shutting down"
                            );
                            break;
                        }
                        tracing::trace!(finalized_height = %height, "Updated cached finalized header");
                        {
                            let recent_header_read =
                                recent_headers.read().expect(RECENT_HEADERS_POISONED);
                            if !recent_header_read.contains_key(&height) {
                                drop(recent_header_read);
                                let mut recent_header_write =
                                    recent_headers.write().expect(RECENT_HEADERS_POISONED);
                                recent_header_write.insert(height, finalized_header);
                                if recent_header_write.len() > MAX_RECENT_HEADERS {
                                    let evicted = recent_header_write.pop_first();
                                    tracing::trace!(?evicted, "Evicting older header");
                                }
                                tracing::trace!(finalized_height = %height, "Updated cached recent headers");
                            }
                        }
                        tracing::trace!(finalized_height = %height, "Updated cache of recent headers");
                    }
                    FutureOrShutdownOutput::Output(Err(error)) => {
                        // DaService should do all retries, so we just stop and fail.
                        tracing::warn!(
                            ?error,
                            "Failed to fetch finalized header, stopping the task"
                        );
                        break;
                    }
                }
            }
        }
    }

    tracing::info!("DA finalized header provider background task stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use sov_mock_da::storable::StorableMockDaService;
    use std::time::Duration;

    #[tokio::test]
    async fn test_last_finalized_header_is_cached() -> anyhow::Result<()> {
        let da_service = StorableMockDaService::new_in_memory(Default::default(), 0).await;

        for _ in 0..5 {
            da_service.send_transaction(&[1; 32]).await.await??;
        }

        let (sender, receiver) = tokio::sync::watch::channel(());
        let da_service = Arc::new(da_service);
        let cache = DaServiceWithCachedFinalizedHeaders::new(
            da_service.clone(),
            receiver,
            Duration::from_millis(300),
        )
        .await?;

        // This should be height 6
        da_service.send_transaction(&[2; 32]).await.await??;
        // Immediately after, shouldn't be pulled yet.
        // If/when background task will switch to subscription, this test is going to break.
        let cached_header = cache.get_last_finalized_block_header()?;
        assert_eq!(cached_header.height(), 5);
        tokio::time::sleep(Duration::from_millis(600)).await;
        let cached_header = cache.get_last_finalized_block_header()?;
        assert_eq!(cached_header.height(), 6);
        da_service.send_transaction(&[3; 32]).await.await??;

        sender.send(())?;
        Ok(())
    }

    #[tokio::test]
    async fn test_recent_headers_are_cached() -> anyhow::Result<()> {
        let da_service = StorableMockDaService::new_in_memory(Default::default(), 0).await;

        for _ in 0..10 {
            da_service.send_transaction(&[1; 32]).await.await??;
        }

        let (sender, receiver) = tokio::sync::watch::channel(());
        let da_service = Arc::new(da_service);
        let cache = DaServiceWithCachedFinalizedHeaders::new(
            da_service.clone(),
            receiver,
            Duration::from_millis(100),
        )
        .await?;

        tokio::time::sleep(Duration::from_millis(200)).await;

        let header_5 = cache.get_block_header_at(5).await?;
        assert_eq!(header_5.height(), 5);

        let header_9 = cache.get_block_header_at(9).await?;
        assert_eq!(header_9.height(), 9);

        sender.send(())?;
        Ok(())
    }

    #[tokio::test]
    async fn test_cache_eviction() -> anyhow::Result<()> {
        let da_service = StorableMockDaService::new_in_memory(Default::default(), 0).await;

        // Produce more blocks than MAX_RECENT_HEADERS
        for _ in 0..40 {
            da_service.send_transaction(&[1; 32]).await.await??;
        }

        let (sender, receiver) = tokio::sync::watch::channel(());
        let da_service = Arc::new(da_service);
        let cache = DaServiceWithCachedFinalizedHeaders::new(
            da_service.clone(),
            receiver,
            Duration::from_millis(100),
        )
        .await?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        // Cache should have at most MAX_RECENT_HEADERS entries
        let cache_size = cache
            .recent_headers
            .read()
            .expect(RECENT_HEADERS_POISONED)
            .len();
        assert!(
            cache_size <= MAX_RECENT_HEADERS,
            "Cache size {cache_size} exceeds MAX_RECENT_HEADERS {MAX_RECENT_HEADERS}"
        );

        sender.send(())?;
        Ok(())
    }

    #[tokio::test]
    async fn test_shutdown_stops_background_task() -> anyhow::Result<()> {
        let da_service = StorableMockDaService::new_in_memory(Default::default(), 0).await;

        da_service.send_transaction(&[1; 32]).await.await??;

        let (sender, receiver) = tokio::sync::watch::channel(());
        let da_service = Arc::new(da_service);
        let cache = DaServiceWithCachedFinalizedHeaders::new(
            da_service.clone(),
            receiver,
            Duration::from_millis(100),
        )
        .await?;

        // Task should be running
        assert!(!cache.finalized_headers_task.is_finished());

        sender.send(())?;

        // Wait a bit for the task to finish
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(cache.finalized_headers_task.is_finished());

        Ok(())
    }

    #[tokio::test]
    async fn test_get_header_at_falls_back_to_da_service() -> anyhow::Result<()> {
        let da_service = StorableMockDaService::new_in_memory(Default::default(), 0).await;

        for _ in 0..5 {
            da_service.send_transaction(&[1; 32]).await.await??;
        }

        let (sender, receiver) = tokio::sync::watch::channel(());
        let da_service = Arc::new(da_service);
        let cache = DaServiceWithCachedFinalizedHeaders::new(
            da_service.clone(),
            receiver,
            Duration::from_millis(100),
        )
        .await?;

        // Request a header that's not in cache - should fall back to DA service
        let header_0 = cache.get_block_header_at(0).await?;
        assert_eq!(header_0.height(), 0);

        sender.send(())?;
        Ok(())
    }

    #[tokio::test]
    async fn test_task_failure_is_detected() -> anyhow::Result<()> {
        let da_service = StorableMockDaService::new_in_memory(Default::default(), 0).await;

        da_service.send_transaction(&[1; 32]).await.await??;

        let (sender, receiver) = tokio::sync::watch::channel(());
        let da_service = Arc::new(da_service);
        let cache = DaServiceWithCachedFinalizedHeaders::new(
            da_service.clone(),
            receiver,
            Duration::from_millis(100),
        )
        .await?;

        // Abort the background task to simulate failure
        cache.finalized_headers_task.abort();

        // Wait for it to finish
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Trying to get the last finalized header should now fail
        let result = cache.get_last_finalized_block_header();
        assert!(
            result.is_err(),
            "Expected error when background task is stopped"
        );

        let _ = sender.send(());
        Ok(())
    }
}
