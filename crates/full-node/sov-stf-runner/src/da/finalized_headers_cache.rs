//! Provides finalized headers

use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use std::collections::BTreeMap;
use std::sync::Arc;

const MAX_RECENT_HEADERS: usize = 30;

/// Wrapper around [`DaService`] that optimizes interaction with actual DA:
///  * Providing access to the last finalized header without actual network call
///  * Storing N recent headers to reduce network calls
#[derive(Debug)]
pub struct DaServiceWithCachedFinalizedHeaders<Da: DaService> {
    da_service: Arc<Da>,
    last_finalized: tokio::sync::watch::Receiver<<Da::Spec as DaSpec>::BlockHeader>,
    recent_headers: Arc<tokio::sync::RwLock<BTreeMap<u64, <Da::Spec as DaSpec>::BlockHeader>>>,
    finalized_headers_task: tokio::task::JoinHandle<()>,
}

impl<Da: DaService> DaServiceWithCachedFinalizedHeaders<Da> {
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

        let recent_headers = Arc::new(tokio::sync::RwLock::new(BTreeMap::new()));

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
            finalized_headers_task: finalized_header_handler,
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
        let cache = self.recent_headers.read().await;
        if let Some(cached_header) = cache.get(&height).cloned() {
            return Ok(cached_header);
        }
        drop(cache);
        self.da_service.get_block_header_at(height).await
    }
}

// TODO: Switch to subscription when it is brought back
async fn background_header_fetch_task<Da: DaService>(
    da_service: std::sync::Arc<Da>,
    finalized_sender: tokio::sync::watch::Sender<<Da::Spec as DaSpec>::BlockHeader>,
    recent_headers: std::sync::Arc<
        tokio::sync::RwLock<BTreeMap<u64, <Da::Spec as DaSpec>::BlockHeader>>,
    >,
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
                            let recent_header_read = recent_headers.read().await;
                            if !recent_header_read.contains_key(&height) {
                                drop(recent_header_read);
                                let mut recent_header_write = recent_headers.write().await;
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
    // use super::*;

    // TODO
}
