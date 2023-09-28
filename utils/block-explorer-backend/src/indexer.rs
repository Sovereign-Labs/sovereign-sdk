use std::time::Duration;

use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::rpc::{
    ItemOrHash, LedgerRpcProvider, QueryMode, SlotResponse, TxResponse,
};
use tracing::{trace, warn};

use crate::AppState;

fn get_txs_from_slot_response(
    slot: &SlotResponse<SequencerOutcome<CelestiaAddress>, TxEffect>,
) -> Vec<TxResponse<TxEffect>> {
    slot.batches
        .as_ref()
        .unwrap()
        .into_iter()
        .map(|x| match x {
            ItemOrHash::Hash(_) => panic!("query mode is not full"),
            ItemOrHash::Full(item) => item,
        })
        .map(|batch| batch.txs.clone().unwrap_or_default())
        .flatten()
        .map(|x| match x {
            ItemOrHash::Hash(_) => panic!("query mode is not full"),
            ItemOrHash::Full(item) => item,
        })
        .collect::<Vec<_>>()
}

/// Indexing workflow:
/// - Get the chain head.
/// - If you are behind, start fetching blocks from the last indexed block.
/// - For each block, insert it into the database.
/// - For each transaction, insert it into the database.
/// Repeat.
pub async fn index_blocks_loop(app_state: AppState, polling_interval: Duration) {
    loop {
        index_blocks(app_state.clone(), polling_interval).await;
    }
}

pub async fn index_blocks(app_state: AppState, polling_interval: Duration) {
    type B = SequencerOutcome<CelestiaAddress>;
    type Tx = TxEffect;

    trace!(
        polling_interval_in_msecs = polling_interval.as_millis(),
        "Going to sleep before new polling cycle"
    );
    // Sleep for a bit. We wouldn't want to spam the node.
    tokio::time::sleep(polling_interval).await;

    // TODO: retry and error handling.
    let chain_head = if let Some(head) = app_state.rpc.get_head::<B, Tx>(QueryMode::Full).unwrap() {
        head
    } else {
        warn!("`get_head` returned no data, can't index blocks.");
        return;
    };

    app_state
        .db
        .insert_chain_head(&serde_json::to_value(&chain_head).unwrap())
        .await
        .unwrap();

    // FIXME: slot n. 0 is nonexistent, but it's probably a bug in the node's
    // JSON-RPC.
    for i in 1..chain_head.number {
        // TODO: batch requests to improve performance and reduce load on
        // the node.
        let block = app_state
            .rpc
            .get_slot_by_number::<B, Tx>(i, QueryMode::Full)
            .unwrap();
        if let Some(block) = block {
            let txs = get_txs_from_slot_response(&block);
            let block_json = serde_json::to_value(block).unwrap();
            app_state.db.upsert_blocks(&[block_json]).await.unwrap();
            let txs_json = txs
                .into_iter()
                .map(|tx| serde_json::to_value(tx).unwrap())
                .collect::<Vec<_>>();
            app_state.db.upsert_transactions(&txs_json).await.unwrap();
        } else {
            warn!("`get_slot_by_number` returned no data for block {}", i);
        }
    }

    // Finally, insert the chain head.
    let chain_head_json = serde_json::to_value(chain_head).unwrap();
    app_state
        .db
        .upsert_blocks(&[chain_head_json])
        .await
        .unwrap();
}
