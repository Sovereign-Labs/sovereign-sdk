//! Helper utilities for interacting with the DA layer.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::node::da::{DaService, SlotData};
use sov_rollup_interface::node::{future_or_shutdown, DaSyncState, FutureOrShutdownOutput};

const MAX_GET_BLOCK_ATTEMPTS: u32 = 10;

/// Tries to fetch block at given height.
/// If `DaSyncState.target_height` becomes lower in the case of re-org, the function fetches a new head instead.
/// Because DaSyncState is polling target height periodically,
/// there is a possibility that this function won't notice change in target height if the polling interval of DaSyncState is too high.
/// To mitigate this, it retries to call `get_block_at` several times before giving up and returning an error.
pub(crate) async fn fetch_block_reorg_aware<Da: DaService>(
    da_service: &Da,
    sync_state: &DaSyncState,
    height: u64,
    polling_interval: Duration,
    da_total_timeout: Duration,
) -> anyhow::Result<Da::FilteredBlock> {
    tracing::trace!(
        height,
        ?polling_interval,
        total_timeout = ?da_total_timeout,
        "Fetch polling for a block"
    );
    let mut requested_height = height;
    let mut interval = tokio::time::interval(polling_interval);

    let check_height = |h| -> u64 {
        let target_height = sync_state.target_da_height.load(Ordering::Relaxed);
        // Allow requesting height next after head, this is a normal operation.
        let highest_allowed_to_request = target_height.saturating_add(1);

        if highest_allowed_to_request < h {
            tracing::info!(
                new_head_height = target_height,
                h,
                "Head height decreased below currently requesting, re-requesting at new head"
            );
            target_height
        } else {
            h
        }
    };

    let mut attempt = 0;
    let sleep = tokio::time::sleep(da_total_timeout);
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            result = da_service.get_block_at(requested_height) => {
                tracing::trace!(
                    requested_height,
                    original_height = height,
                    is_err = result.is_err(),
                    attempt,
                    "Received result from `get_block_at`");
                match result {
                    Ok(block) => {
                        tracing::trace!(block_header = %block.header().display(), "Block fetched, returning");
                        return Ok(block);
                    }
                    Err(err) => {
                        tracing::trace!(?err, requested_height, attempt, "Error fetching block");
                        attempt += 1;
                        let requestable_height = check_height(requested_height);
                        if requestable_height != requested_height {
                            tracing::info!(requestable_height, "Request able height has changed, trying again");
                            requested_height = requestable_height;
                            continue;
                        } else if attempt >= MAX_GET_BLOCK_ATTEMPTS {
                            anyhow::bail!("Failed to fetch block after {MAX_GET_BLOCK_ATTEMPTS} attempts. Last error: {:?}", err);
                        } else {
                            // What if the target height is not updated, and we've returning early.
                            // Basically we should note that if the polling interval is more than (block_time * attempts) it will error in case of rewind.
                            tracing::info!(requestable_height, attempt, "Height hasn't changed, retrying again.");
                        }
                    }
                }
            }
            _ = interval.tick() => {
                requested_height = check_height(requested_height);
            }
            _ = &mut sleep => {
                anyhow::bail!("Total timeout after {:?} while trying fetching block at height {}", da_total_timeout, requested_height);
            }
        }
    }
}

/// Helper struct that makes getting finalized block headers more efficient, by caching them.
///
/// This provider maintains a background task that periodically polls the DA service for
/// the latest head and finalized block headers, caching them for efficient access.
#[derive(Clone)]
pub struct DaFinalizedHeaderProvider<Da: DaService> {
    da_service: std::sync::Arc<Da>,
    head: tokio::sync::watch::Receiver<<Da::Spec as DaSpec>::BlockHeader>,
    last_finalized: tokio::sync::watch::Receiver<<Da::Spec as DaSpec>::BlockHeader>,
    is_background_running: std::sync::Arc<AtomicBool>,
}

impl<Da: DaService> DaFinalizedHeaderProvider<Da> {
    /// Returns the current cached head block header.
    ///
    /// # Errors
    ///
    /// Returns an error if the background polling task has stopped.
    pub fn get_head(&self) -> anyhow::Result<<Da::Spec as DaSpec>::BlockHeader> {
        if !self.is_background_running.load(Ordering::Acquire) {
            anyhow::bail!("DA header provider background task has stopped");
        }
        Ok(self.head.borrow().clone())
    }

    /// Returns the current cached last finalized block header.
    ///
    /// # Errors
    ///
    /// Returns an error if the background polling task has stopped.
    pub fn get_last_finalized(&self) -> anyhow::Result<<Da::Spec as DaSpec>::BlockHeader> {
        if !self.is_background_running.load(Ordering::Acquire) {
            anyhow::bail!("DA header provider background task has stopped");
        }
        Ok(self.last_finalized.borrow().clone())
    }

    /// Fetches a block header at the specified height from the DA service.
    ///
    /// This is a direct passthrough to the underlying DA service.
    pub async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Da::Spec as DaSpec>::BlockHeader, Da::Error> {
        self.da_service.get_block_header_at(height).await
    }
}

/// Initializes a DA header provider with a background polling task.
///
/// This function fetches the initial head and finalized headers, then spawns a background
/// task to continuously poll for updates at the specified interval. The background task
/// will automatically stop when the `shutdown_rx` channel signals shutdown or when all
/// provider instances are dropped.
///
/// # Arguments
///
/// * `da_service` - The DA service to poll for headers
/// * `polling_interval` - How frequently to poll for new headers
/// * `shutdown_rx` - Watch receiver that signals when to shutdown the background task
///
/// # Returns
///
/// Returns a `DaFinalizedHeaderProvider` which can be cloned and used to access cached headers.
///
/// # Errors
///
/// Returns an error if the initial header fetch fails or if `polling_interval` is zero.
pub async fn initialize_da_header_provider<Da: DaService>(
    da_service: std::sync::Arc<Da>,
    polling_interval: std::time::Duration,
    shutdown_rx: tokio::sync::watch::Receiver<()>,
) -> anyhow::Result<DaFinalizedHeaderProvider<Da>> {
    // Validate parameters
    if polling_interval.is_zero() {
        anyhow::bail!("polling_interval must be greater than zero");
    }

    // Fetch initial headers with proper error handling
    let head_header = da_service
        .get_head_block_header()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch initial head header: {:?}", e))?;

    let finalized_header = da_service
        .get_last_finalized_block_header()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch initial finalized header: {:?}", e))?;

    tracing::info!(
        head_height = %head_header.height(),
        finalized_height = %finalized_header.height(),
        "Initialized DA header provider"
    );

    let (head_sender, head_receiver) = tokio::sync::watch::channel(head_header);
    let (finalized_sender, finalized_receiver) = tokio::sync::watch::channel(finalized_header);

    let own_da_service = da_service.clone();

    let is_running_for_reader = std::sync::Arc::new(AtomicBool::new(true));
    let is_running_for_writer = is_running_for_reader.clone();

    let _handle = tokio::task::spawn(async move {
        background_header_fetch_task(
            da_service,
            head_sender,
            finalized_sender,
            polling_interval,
            shutdown_rx,
            is_running_for_writer,
        )
        .await;
    });

    let provider = DaFinalizedHeaderProvider {
        da_service: own_da_service,
        head: head_receiver,
        last_finalized: finalized_receiver,
        is_background_running: is_running_for_reader,
    };

    Ok(provider)
}

async fn background_header_fetch_task<Da: DaService>(
    da_service: std::sync::Arc<Da>,
    head_sender: tokio::sync::watch::Sender<<Da::Spec as DaSpec>::BlockHeader>,
    finalized_sender: tokio::sync::watch::Sender<<Da::Spec as DaSpec>::BlockHeader>,
    polling_interval: std::time::Duration,
    shutdown_rx: tokio::sync::watch::Receiver<()>,
    is_running_for_writer: std::sync::Arc<AtomicBool>,
) {
    let mut interval = tokio::time::interval(polling_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Wait for next tick or shutdown
        match future_or_shutdown(interval.tick(), &shutdown_rx).await {
            FutureOrShutdownOutput::Shutdown => {
                tracing::info!("DA header provider received shutdown signal");
                break;
            }
            FutureOrShutdownOutput::Output(_) => {
                // Fetch head header
                match future_or_shutdown(da_service.get_head_block_header(), &shutdown_rx).await {
                    FutureOrShutdownOutput::Shutdown => {
                        tracing::info!("DA header provider received shutdown signal");
                        break;
                    }
                    FutureOrShutdownOutput::Output(Ok(head_header)) => {
                        let height = head_header.height();
                        if head_sender.send(head_header).is_err() {
                            tracing::debug!(
                                "All DA header provider receivers dropped, shutting down"
                            );
                            break;
                        }
                        tracing::trace!(head_height = %height, "Updated cached head header");
                    }
                    FutureOrShutdownOutput::Output(Err(e)) => {
                        tracing::warn!(
                            ?e,
                            "Failed to fetch head header, will retry on next interval"
                        );
                        continue;
                    }
                }

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
                        if finalized_sender.send(finalized_header).is_err() {
                            tracing::debug!(
                                "All DA header provider receivers dropped, shutting down"
                            );
                            break;
                        }
                        tracing::trace!(finalized_height = %height, "Updated cached finalized header");
                    }
                    FutureOrShutdownOutput::Output(Err(e)) => {
                        tracing::warn!(
                            ?e,
                            "Failed to fetch finalized header, will retry on next interval"
                        );
                        // Don't continue here - we already updated head, which is acceptable
                    }
                }
            }
        }
    }

    is_running_for_writer.store(false, Ordering::Release);
    tracing::info!("DA header provider background task stopped");
}
