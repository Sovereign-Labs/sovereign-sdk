//! Helper utilities for interacting with the DA layer.
use std::sync::atomic::Ordering;
use std::time::Duration;

use sov_rollup_interface::da::BlockHeaderTrait;
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
) -> anyhow::Result<Da::FilteredBlock> {
    tracing::trace!(height, "Fetch polling for a block");
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
    loop {
        tokio::select! {
            result = da_service.get_block_at(requested_height) => {
                tracing::trace!(
                    requested_height,
                    original_height = height,
                    is_err = result.is_err(),
                    attempt,
                    "received result from `get_block_at`");
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
        }
    }
}
