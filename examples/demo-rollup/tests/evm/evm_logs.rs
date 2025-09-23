use crate::evm::evm_test_helper::setup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use alloy_primitives::B256;
use alloy_primitives::U256;
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter};
use sov_sequencer::SeqConfigExtension;

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs() {
    let (test_rollup, evm_client, _) = setup(0, EVM_EXTENSION).await;
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

    {
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

    // topic3 seolects one log from each tx
    {
        let topic: B256 = U256::from(3).into();
        let filter = Filter::new().at_block_hash(block_hash).topic3(topic);

        let logs = evm_client.get_logs(&filter).await;
        check_logs(&filter, logs, nb_of_txs);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_range() {
    let (test_rollup, evm_client, _) = setup(0, EVM_EXTENSION).await;
    let contract_address = evm_client.alloy_deploy_contract().await;
    test_rollup.wait_for_next_blocks(1).await;

    let nb_of_txs = 10;
    let nb_of_logs_per_tx = 5;

    let start_block = evm_client
        .alloy_get_block_by_number(Some(BlockNumberOrTag::Latest.to_string()))
        .await
        .number();

    let mut tx_hashes = Vec::new();
    for i in 0..nb_of_txs {
        let hash = evm_client
            .alloy_emit_logs(contract_address, i, nb_of_logs_per_tx)
            .await;
        tx_hashes.push(hash);
        if i % 3 == 0 {
            test_rollup.wait_for_next_blocks(1).await;
        }
    }
    test_rollup.wait_for_next_blocks(1).await;

    // Check logs from all txs.
    {
        let filter = Filter::new()
            .from_block(start_block)
            .to_block(BlockNumberOrTag::Latest);

        let logs = evm_client.get_logs(&filter).await;
        check_logs(&filter, logs, nb_of_txs * nb_of_logs_per_tx);
    }

    // topic3 seolects one log from each tx
    {
        let topic: B256 = U256::from(3).into();
        let filter = Filter::new()
            .from_block(start_block)
            .to_block(BlockNumberOrTag::Latest)
            .topic3(topic);

        let logs = evm_client.get_logs(&filter).await;
        check_logs(&filter, logs, nb_of_txs);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_range_limit() {
    let max_log_limit = 93;

    let (test_rollup, evm_client, _) = setup(0, SeqConfigExtension { max_log_limit }).await;
    let contract_address = evm_client.alloy_deploy_contract().await;
    test_rollup.wait_for_next_blocks(1).await;

    let nb_of_txs = 20;
    let nb_of_logs_per_tx: u32 = 5;

    let start_block = evm_client
        .alloy_get_block_by_number(Some(BlockNumberOrTag::Latest.to_string()))
        .await
        .number();

    let mut tx_hashes = Vec::new();
    for i in 0..nb_of_txs {
        let hash = evm_client
            .alloy_emit_logs(contract_address, i, nb_of_logs_per_tx)
            .await;
        tx_hashes.push(hash);
        if i % 3 == 0 {
            test_rollup.wait_for_next_blocks(1).await;
        }
    }
    test_rollup.wait_for_next_blocks(1).await;

    let filter = Filter::new()
        .from_block(start_block - 2)
        .to_block(BlockNumberOrTag::Latest);

    let logs = evm_client.get_logs(&filter).await;

    assert_eq!(logs.len(), max_log_limit);
}

fn check_logs(filter: &Filter, logs: Vec<alloy_rpc_types_eth::Log>, expected_nb_of_logs: u32) {
    assert_eq!(logs.len() as u32, expected_nb_of_logs);
    for log in logs {
        assert!(filter.matches(log.inner.as_ref()));
    }
}
