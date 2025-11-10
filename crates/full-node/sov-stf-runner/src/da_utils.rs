//! Helper utilities for interacting with the DA layer.

use std::sync::atomic::Ordering;
use std::time::Duration;

use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::DaSyncState;

const MAX_GET_BLOCK_ATTEMPTS: u32 = 10;

async fn get_new_head_height_if_roll_back(
    sync_state: &DaSyncState,
    requested_height: u64,
    polling_interval: Duration,
) -> u64 {
    loop {
        let target_height = sync_state.target_da_height.load(Ordering::Relaxed);
        if target_height < requested_height {
            return target_height;
        }
        tokio::time::sleep(polling_interval).await;
    }
}

/// Tries to fetch block at given height.
/// If `DaSyncState.target_height` becomes lower in the case of re-org, the function fetches a new head instead.
/// Because DaSyncState is polling target height periodically,
/// there is a possibility that this function won't notice change in target height if the polling interval of DaSyncState is too high, but that's fine
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

    // During each iteration of the loop, it will try to fetch da block at given height
    // If head rolls back below requested,
    // requested height will be changed to height and new iteration of the loop continues
    // We allow chain to consecutively rewind of `MAX_GET_BLOCK_ATTEMPTS` after that it will error
    for attempt in 1..=MAX_GET_BLOCK_ATTEMPTS {
        // If the head rolled back below requested height, we will try to request the head instead.
        let rolled_back_head_future =
            get_new_head_height_if_roll_back(sync_state, height, polling_interval);
        // DaService suppose to do all necessary retries on failures.
        // Runner only limits the total time of this activity.
        let get_block_future =
            tokio::time::timeout(da_total_timeout, da_service.get_block_at(requested_height));
        tokio::select! {
            get_block_result = get_block_future => {
                // Just flatten timeout, but return as is
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
                    ouf_of_attempts = MAX_GET_BLOCK_ATTEMPTS,
                    "DA head has rolled back below requested height. Updating requested height and retrying");
                requested_height = rolled_back_height;
            }
        }
    }

    anyhow::bail!("Failed to fetch block after {MAX_GET_BLOCK_ATTEMPTS} of attempts");
}
