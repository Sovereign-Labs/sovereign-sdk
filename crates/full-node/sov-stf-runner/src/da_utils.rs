#![allow(dead_code)]
//! Helper utilities for interacting with the DA layer.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::node::da::{DaService, SlotData};
use sov_rollup_interface::node::DaSyncState;

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

#[derive(Clone)]
pub struct DaHeaderProvider<Da: DaSpec> {
    head: tokio::sync::watch::Receiver<Da::BlockHeader>,
    last_finalized: tokio::sync::watch::Receiver<Da::BlockHeader>,
    is_running: std::sync::Arc<AtomicBool>,
}

impl<Da: DaSpec> DaHeaderProvider<Da> {
    pub fn get_head(&self) -> anyhow::Result<Da::BlockHeader> {
        // TODO: Check if is running and return error
        Ok(self.head.borrow().clone())
    }

    pub fn get_last_finalize(&self) -> anyhow::Result<Da::BlockHeader> {
        // TODO: Check if is running and return error
        Ok(self.last_finalized.borrow().clone())
    }

    // TODO: Can be used
    // pub fn subscribe_to_head(&self) ->
}

pub async fn initialize_da_header_provider<Da: DaService>(
    da_service: std::sync::Arc<Da>,
    polling_interval: std::time::Duration,
) -> anyhow::Result<DaHeaderProvider<Da::Spec>> {
    // TODO: unwraps
    let head_header = da_service.get_head_block_header().await.unwrap();
    let finalized_header = da_service.get_last_finalized_block_header().await.unwrap();
    let (head_sender, head_receiver) = tokio::sync::watch::channel(head_header);
    let (finalized_sender, finalized_receiver) = tokio::sync::watch::channel(finalized_header);

    let is_running_for_reader = std::sync::Arc::new(AtomicBool::new(true));
    let is_running_for_writer = is_running_for_reader.clone();

    let _handle = tokio::task::spawn(async move {
        loop {
            // Naive implementation to see the effect on the main loop.
            // Better to bring back subscriptions and use them
            let Ok(head_header) = da_service.get_head_block_header().await else {
                break;
            };
            if head_sender.send(head_header).is_err() {
                break;
            }
            let Ok(finalized_header) = da_service.get_last_finalized_block_header().await else {
                break;
            };
            if finalized_sender.send(finalized_header).is_err() {
                break;
            }
            tokio::time::sleep(polling_interval).await;
        }

        is_running_for_writer.store(false, Ordering::Release);
    });

    Ok(DaHeaderProvider {
        head: head_receiver,
        last_finalized: finalized_receiver,
        is_running: is_running_for_reader,
    })
}
