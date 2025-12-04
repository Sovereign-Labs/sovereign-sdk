use futures::stream::StreamExt;
use sov_api_spec::types::TxStatus;
use sov_sequencer::Sequencer;
use sov_test_utils::sequencer::TestSequencerSetup;

use crate::utils::{generate_txs, RT};

#[tokio::test(flavor = "multi_thread")]
async fn mempool_eviction_event() {
    let mempool_max_txs_count = 1;
    let sequencer = TestSequencerSetup::<RT>::with_real_sequencer_and_mempool_max_txs_count(
        mempool_max_txs_count.try_into().unwrap(),
    )
    .await
    .unwrap();

    let txs = generate_txs(sequencer.admin_private_key.clone());

    let client = sequencer.client();
    let mut subscription = client
        .subscribe_to_tx_status_updates(txs[0].tx_hash)
        .await
        .unwrap();

    // Before submitting a transaction, its status is unknown.
    assert_eq!(
        subscription.next().await.unwrap().unwrap().status,
        TxStatus::Unknown
    );

    sequencer
        .sequencer
        .accept_tx(txs[0].fully_baked_tx.clone())
        .await
        .unwrap();

    // The transaction enters the mempool.
    assert_eq!(
        subscription.next().await.unwrap().unwrap().status,
        TxStatus::Submitted
    );

    // In the meantime, another transaction enters the mempool and causes the
    // first one to be evicted.
    sequencer
        .sequencer
        .accept_tx(txs[1].fully_baked_tx.clone())
        .await
        .unwrap();

    assert_eq!(
        subscription.next().await.unwrap().unwrap().status,
        TxStatus::Dropped,
    );

    // No more events.
    assert!(subscription.next().await.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn rpc_subscribe() {
    let sequencer = TestSequencerSetup::<RT>::with_real_sequencer()
        .await
        .unwrap();
    let client = sequencer.client();

    let txs = generate_txs(sequencer.admin_private_key.clone());

    let mut subscription = client
        .subscribe_to_tx_status_updates(txs[0].tx_hash)
        .await
        .unwrap();

    // Before submitting a transaction, its status is unknown.
    assert_eq!(
        subscription.next().await.unwrap().unwrap().status,
        TxStatus::Unknown
    );

    let tx_objs = txs.into_iter().map(|tx| tx.tx_object).collect::<Vec<_>>();
    let _ = client.send_txs_to_sequencer(&tx_objs).await;
    let _ = sequencer.sequencer.produce_and_submit_batch().await;

    // The transaction status should change once it enters the mempool...
    assert_eq!(
        subscription.next().await.unwrap().unwrap().status,
        TxStatus::Submitted
    );

    // ...and then change again shortly after, once it gets included in a block.
    assert_eq!(
        subscription.next().await.unwrap().unwrap().status,
        TxStatus::Published,
    );

    // TODO: finalized status (https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1088).

    // No more events.
    //assert!(subscription.next().await.is_none());
}
