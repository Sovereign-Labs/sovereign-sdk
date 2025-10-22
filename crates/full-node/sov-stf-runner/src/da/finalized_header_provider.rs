//! Provides finalized headers

use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_RECENT_HEADERS: usize = 30;

// TODO: Switch to subscription when it is brought back

async fn background_header_fetch_task<Da: DaService>(
    da_service: std::sync::Arc<Da>,
    finalized_sender: tokio::sync::watch::Sender<<Da::Spec as DaSpec>::BlockHeader>,
    recent_headers: std::sync::Arc<
        tokio::sync::RwLock<BTreeMap<u64, <Da::Spec as DaSpec>::BlockHeader>>,
    >,
    polling_interval: std::time::Duration,
    shutdown_rx: tokio::sync::watch::Receiver<()>,
    is_running_for_writer: std::sync::Arc<AtomicBool>,
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

    is_running_for_writer.store(false, Ordering::Release);
    tracing::info!("DA header provider background task stopped");
}
