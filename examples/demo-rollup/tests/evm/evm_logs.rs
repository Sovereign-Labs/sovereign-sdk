use alloy_rpc_types_eth::Filter;

use crate::evm::evm_test_helper::setup;

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs() {
    let (test_rollup, evm_client, _) = setup(0).await;
    let contract_address = evm_client.alloy_deploy_contract().await;
    test_rollup.wait_for_next_blocks(1).await;

    // Make sure all the txs are in the same blcok.
    test_rollup.pause_preferred_batches().await;

    let nb_of_txs = 10;
    let nb_of_logs_per_tx = 5;

    let mut tx_hashes = Vec::new();
    for i in 0..nb_of_txs {
        let hash = evm_client
            .alloy_emit_logs(contract_address, i, nb_of_logs_per_tx)
            .await;
        tx_hashes.push(hash);
    }

    test_rollup.resume_preferred_batches().await;
    test_rollup.wait_for_next_blocks(1).await;

    let tx_hash = tx_hashes[0].clone();
    let rec = evm_client.alloy_receipt(tx_hash).await.unwrap();
    let block_hash = rec.block_hash.unwrap();

    let filter = Filter::new().at_block_hash(block_hash);
    let logs = evm_client.get_logs(&filter).await;

    assert_eq!(logs.len() as u32, nb_of_txs * nb_of_logs_per_tx);

    for (index, log) in logs.into_iter().enumerate() {
        let index = index as u64;
        assert!(filter.matches(log.inner.as_ref()));
        assert_eq!(log.log_index.unwrap(), index);
        assert_eq!(
            log.transaction_index.unwrap(),
            (index / nb_of_logs_per_tx as u64)
        );
    }
}
