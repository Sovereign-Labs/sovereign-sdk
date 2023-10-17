use std::time::Duration;

use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_ledger_rpc::client::RpcClient;
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::rpc::{
    EventIdentifier, ItemOrHash, QueryMode, SlotIdentifier, SlotResponse, TxResponse,
};
use tracing::{debug, error, warn};

use crate::api_v0::models::Event;
use crate::AppState;

type B = SequencerOutcome<CelestiaAddress>;
type Tx = TxEffect;

fn get_txs_from_slot_response(
    slot: &SlotResponse<SequencerOutcome<CelestiaAddress>, TxEffect>,
) -> anyhow::Result<Vec<TxResponse<TxEffect>>> {
    Ok(slot
        .batches
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no batches in slot response"))?
        .iter()
        .map(|x| match x {
            ItemOrHash::Hash(_) => panic!("query mode is not full"),
            ItemOrHash::Full(item) => item,
        })
        .flat_map(|batch| batch.txs.clone().unwrap_or_default())
        .map(|x| match x {
            ItemOrHash::Hash(_) => panic!("query mode is not full"),
            ItemOrHash::Full(item) => item,
        })
        .collect::<Vec<_>>())
}

/// Indexing workflow:
/// - Get the chain head.
/// - If you are behind, start fetching blocks from the last indexed block.
/// - For each block, insert it into the database.
/// - For each transaction, insert it into the database.
/// Repeat.
pub async fn index_blocks_loop(app_state: AppState, polling_interval: Duration) {
    loop {
        // Errors are logged but otherwise ignored. We wouldn't want want to crash the
        // indexer if e.g. the node is down.
        if let Err(err) = index_blocks(app_state.clone(), polling_interval).await {
            error!(%err, "Main indexing loop failed; retrying shortly");
        }
    }
}

pub async fn index_blocks(app_state: AppState, polling_interval: Duration) -> anyhow::Result<()> {
    debug!(
        polling_interval_in_msecs = polling_interval.as_millis(),
        "Going to sleep before new polling cycle"
    );
    // Sleep for a bit. We wouldn't want to spam the node.
    tokio::time::sleep(polling_interval).await;

    // TODO: retry and error handling.
    let chain_head: SlotResponse<B, Tx> =
        if let Some(head) = app_state.rpc().get_head(QueryMode::Full).await? {
            head
        } else {
            warn!("`get_head` returned no data, can't index blocks.");
            return Ok(());
        };

    app_state
        .db
        .begin_transaction()
        .await?
        .insert_chain_head(&serde_json::to_value(&chain_head).unwrap())
        .await?;

    // FIXME: slot n. 0 is nonexistent, but it's probably a bug in the node's
    // JSON-RPC.
    for i in 1..chain_head.number {
        // TODO: batch requests to improve performance and reduce load on
        // the node.
        index_block(app_state.clone(), i).await?;
    }

    // Finally, insert the chain head.
    let chain_head_json =
        serde_json::to_value(chain_head).expect("chain head is not serializable, this is a bug");
    app_state
        .db
        .begin_transaction()
        .await?
        .upsert_blocks(&[chain_head_json])
        .await?;

    Ok(())
}

async fn index_block(app_state: AppState, block_num: u64) -> anyhow::Result<()> {
    let blocks = app_state
        .rpc()
        .get_slots(vec![SlotIdentifier::Number(block_num)], QueryMode::Full)
        .await?;

    let Some(Some(block)) = blocks.first() else {
        warn!(
            "`get_slot_by_number` returned no data for block {}",
            block_num
        );
        return Ok(());
    };

    let txs = get_txs_from_slot_response(block)?;
    let block_json = serde_json::to_value(block).expect("block is not serializable, this is a bug");
    let txs_json = txs
        .iter()
        .map(|tx| serde_json::to_value(tx).expect("tx is not serializable, this is a bug"))
        .collect::<Vec<_>>();
    let batch_range = block.batch_range.clone();
    let batches = app_state
        .rpc()
        .get_batches_range(batch_range.start, batch_range.end, QueryMode::Standard)
        .await?
        .into_iter()
        .map(|batch_opt| serde_json::to_value(batch_opt.expect("No batch")).unwrap())
        .collect::<Vec<_>>();

    let mut all_events = vec![];
    for tx in txs {
        let event_ids = tx
            .event_range
            .clone()
            .map(EventIdentifier::Number)
            .collect();
        let events = app_state
            .rpc()
            .get_events(event_ids)
            .await?
            .into_iter()
            .zip(tx.event_range.clone())
            .map(|event_opt| Event {
                id: event_opt.1 as _,
                key: event_opt.0.as_ref().unwrap().key().inner().clone(),
                value: event_opt.0.as_ref().unwrap().value().inner().clone(),
            })
            .collect::<Vec<_>>();
        all_events.push(events);
    }

    let mut db = app_state.db.begin_transaction().await?;
    db.upsert_blocks(&[block_json]).await?;
    db.upsert_transactions(&txs_json).await?;
    for events in all_events {
        db.upsert_events(&events).await?;
    }
    db.upsert_batches(&batches).await?;
    db.commit().await?;

    Ok(())
}
