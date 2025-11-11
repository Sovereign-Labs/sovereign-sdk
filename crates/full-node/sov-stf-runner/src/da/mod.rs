//! Helper utilities for interacting with the DA layer.


mod bulk_finalized_blocks_fetcher;
mod finalized_headers_cache;

pub use bulk_finalized_blocks_fetcher::FinalizedBlocksBulkFetcher;
pub use finalized_headers_cache::DaServiceWithCachedFinalizedHeaders;

use std::sync::atomic::Ordering;
use std::time::Duration;

use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::DaSyncState;

const MAX_GET_BLOCK_ATTEMPTS: u32 = 10;

/// Waits for the DA head to roll back below the requested height.
///
/// This function blocks indefinitely until a reorg occurs. It must be used in `tokio::select!`
/// with another future that will eventually complete (e.g., `get_block_at`).
///
/// Returns the new target height once it drops below `requested_height - 1`.
/// Note: Requesting `target_height + 1` is normal when the node is synced and waiting for the next block.
async fn get_new_head_height_if_roll_back(sync_state: &DaSyncState, requested_height: u64) -> u64 {
    let mut rx = sync_state.sync_status_sender.subscribe();
    loop {
        let status = *rx.borrow_and_update();
        let target_height = status.target_da_height();
        tracing::trace!(?status, "Received SyncStatus update from the channel");
        // Requesting one block ahead of the current head is normal behavior
        // when the node is fully synced and waiting for the next block.
        if target_height.saturating_add(1) < requested_height {
            tracing::trace!(
                requested_height,
                target_height,
                "Received head is below expected, exit condition hit"
            );
            return target_height;
        }
        tracing::trace!(
            requested_height,
            target_height,
            "Received head is in expected range, keep waiting for new value"
        );
        if rx.changed().await.is_err() {
            // Sender dropped, fall back to reading the atomic value
            return sync_state.target_da_height.load(Ordering::Relaxed);
        }
    }
}

/// Fetches a DA block at the given height with reorg awareness.
///
/// If the DA head rolls back during the fetch (reorg detected), this function automatically
/// retries with the new head height. Allows up to `MAX_GET_BLOCK_ATTEMPTS` consecutive reorgs
/// before returning an error.
pub(crate) async fn fetch_block_reorg_aware<Da: DaService>(
    da_service: &Da,
    sync_state: &DaSyncState,
    height: u64,
    da_total_timeout: Duration,
) -> anyhow::Result<Da::FilteredBlock> {
    tracing::trace!(
        height,
        total_timeout = ?da_total_timeout,
        "Fetch polling for a block"
    );

    let mut requested_height = height;

    // Retry up to MAX_GET_BLOCK_ATTEMPTS times if the chain reorgs during fetch.
    // Each iteration races between fetching the block and detecting a reorg.
    for attempt in 1..=MAX_GET_BLOCK_ATTEMPTS {
        let rolled_back_head_future =
            get_new_head_height_if_roll_back(sync_state, requested_height);
        // DaService handles its own retries for transient failures.
        // We only enforce a total timeout per attempt.
        let get_block_future =
            tokio::time::timeout(da_total_timeout, da_service.get_block_at(requested_height));
        tokio::select! {
            get_block_result = get_block_future => {
                match get_block_result {
                    Ok(inner_result) => {
                        return inner_result.map_err(|error| anyhow::anyhow!("Error from DaService: {error:?}"));
                    }
                    Err(_) => {
                        anyhow::bail!("Timeout getting block from DaService after {da_total_timeout:?}");
                    }
                }
            }
            rolled_back_height = rolled_back_head_future => {
                tracing::warn!(
                    requested_height,
                    new_da_head_height = rolled_back_height,
                    attempt,
                    out_of_attempts = MAX_GET_BLOCK_ATTEMPTS,
                    "DA head rolled back below requested height, retrying with new head");
                requested_height = rolled_back_height;
            }
        }
    }

    anyhow::bail!("Failed to fetch block after {MAX_GET_BLOCK_ATTEMPTS} consecutive reorgs");
}
