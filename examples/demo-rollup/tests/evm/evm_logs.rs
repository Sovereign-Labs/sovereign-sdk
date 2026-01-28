#![allow(deprecated)] // Allowed for using alloy things.
use crate::evm::evm_test_helper::setup_with_simple_storage;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use alloy_primitives::{keccak256, Address, TxHash, B256, U256};
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter, Log};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde_json::Value;
use sov_demo_rollup::MockDemoRollup;
use sov_eth_client::SimpleStorageClient;
use sov_ethereum::Cursor;
use sov_modules_api::execution_mode::Native;
use sov_rpc_eth_types::FilterWithCursor;
use sov_rpc_eth_types::LogsWithMaybeCursor;
use sov_sequencer::SeqConfigExtension;
use sov_test_utils::test_rollup::TestRollup;
use std::collections::HashMap;

/// Tests that logs can be retrieved from pending block snapshots.
///
/// Each transaction in the pending block creates a synthetic block hash that represents
/// a snapshot of the pending state at that point. This test verifies:
///
/// 1. **Snapshot creation**: After each tx, a unique synthetic block hash is generated
/// 2. **Query by hash**: Querying by a snapshot's hash returns all logs up to that point
/// 3. **Query by Pending**: Querying with `Pending` tag returns the current (latest) state
/// 4. **Hash and Pending match**: After each tx submission, querying by current block hash of the latest tx equals querying by Pending
/// 5. **Snapshot preservation**: Earlier snapshots remain queryable even after new txs are added, by older hash
/// 6. **Cumulative logs**: Each snapshot contains all logs in the pending block up to that point
///
/// Test flow:
/// - first_tx emits 2 logs → snapshot has 2 logs
/// - second_tx emits 1 log → snapshot has 3 logs (cumulative)
/// - third_tx emits 1 log → snapshot has 4 logs (cumulative)
/// - Final verification: all 3 snapshots are still independently queryable
#[tokio::test(flavor = "multi_thread")]
async fn get_log_from_pending_block() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract_address = client.alloy_deploy_contract().await;
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let sender = client.address();
    let pending_filter = filter_for_tag(BlockNumberOrTag::Pending);

    // Helper to build expected logs for a snapshot
    let build_expected_at_snapshot = |block: &BlockContext,
                                      first_tx: TxHash,
                                      second_tx: Option<TxHash>,
                                      third_tx: Option<TxHash>|
     -> Vec<ExpectedSimpleLog> {
        let mut logs = vec![
            ExpectedSimpleLog {
                meta: ExpectedLogMeta {
                    address: contract_address,
                    tx_hash: first_tx,
                    tx_index: 0,
                    log_index: 0,
                    block_hash: block.hash,
                    block_number: block.number,
                    block_timestamp: Some(block.timestamp),
                },
                topic1: U256::ZERO,
                topic2: U256::ZERO,
                data: U256::ZERO,
            },
            ExpectedSimpleLog {
                meta: ExpectedLogMeta {
                    address: contract_address,
                    tx_hash: first_tx,
                    tx_index: 0,
                    log_index: 1,
                    block_hash: block.hash,
                    block_number: block.number,
                    block_timestamp: Some(block.timestamp),
                },
                topic1: U256::ZERO,
                topic2: U256::from(1),
                data: U256::ZERO,
            },
        ];
        if let Some(tx) = second_tx {
            logs.push(ExpectedSimpleLog {
                meta: ExpectedLogMeta {
                    address: contract_address,
                    tx_hash: tx,
                    tx_index: 1,
                    log_index: 2,
                    block_hash: block.hash,
                    block_number: block.number,
                    block_timestamp: Some(block.timestamp),
                },
                topic1: U256::from(1),
                topic2: U256::ZERO,
                data: U256::ZERO,
            });
        }
        if let Some(tx) = third_tx {
            logs.push(ExpectedSimpleLog {
                meta: ExpectedLogMeta {
                    address: contract_address,
                    tx_hash: tx,
                    tx_index: 2,
                    log_index: 3,
                    block_hash: block.hash,
                    block_number: block.number,
                    block_timestamp: Some(block.timestamp),
                },
                topic1: U256::from(2),
                topic2: U256::ZERO,
                data: U256::ZERO,
            });
        }
        logs
    };

    // After first_tx: 2 logs
    let first_tx = client.alloy_emit_logs(contract_address, 0, 2).await;
    let pending_block_first = latest_block_context(&client).await;
    let expected_at_first = build_expected_at_snapshot(&pending_block_first, first_tx, None, None);

    let logs_by_hash_first = client
        .get_logs(&Filter::new().at_block_hash(pending_block_first.hash))
        .await;
    let logs_by_pending_first = client.get_logs(&pending_filter).await;
    assert_expected_simple_logs(&logs_by_hash_first, &expected_at_first, sender);
    assert_expected_simple_logs(&logs_by_pending_first, &expected_at_first, sender);

    // After second_tx: 3 logs (cumulative)
    let second_tx = client.alloy_emit_logs(contract_address, 1, 1).await;
    let pending_block_second = latest_block_context(&client).await;
    let expected_at_second =
        build_expected_at_snapshot(&pending_block_second, first_tx, Some(second_tx), None);

    let logs_by_hash_second = client
        .get_logs(&Filter::new().at_block_hash(pending_block_second.hash))
        .await;
    let logs_by_pending_second = client.get_logs(&pending_filter).await;
    assert_expected_simple_logs(&logs_by_hash_second, &expected_at_second, sender);
    assert_expected_simple_logs(&logs_by_pending_second, &expected_at_second, sender);

    // After third_tx: 4 logs (cumulative)
    let third_tx = client.alloy_emit_logs(contract_address, 2, 1).await;
    let pending_block_third = latest_block_context(&client).await;
    let expected_at_third = build_expected_at_snapshot(
        &pending_block_third,
        first_tx,
        Some(second_tx),
        Some(third_tx),
    );

    let logs_by_hash_third = client
        .get_logs(&Filter::new().at_block_hash(pending_block_third.hash))
        .await;
    let logs_by_pending_third = client.get_logs(&pending_filter).await;
    assert_expected_simple_logs(&logs_by_hash_third, &expected_at_third, sender);
    assert_expected_simple_logs(&logs_by_pending_third, &expected_at_third, sender);

    // Final snapshot queries: verify each hash still returns its snapshot
    let final_logs_first = client
        .get_logs(&Filter::new().at_block_hash(pending_block_first.hash))
        .await;
    let final_logs_second = client
        .get_logs(&Filter::new().at_block_hash(pending_block_second.hash))
        .await;
    let final_logs_third = client
        .get_logs(&Filter::new().at_block_hash(pending_block_third.hash))
        .await;
    assert_expected_simple_logs(&final_logs_first, &expected_at_first, sender);
    assert_expected_simple_logs(&final_logs_second, &expected_at_second, sender);
    assert_expected_simple_logs(&final_logs_third, &expected_at_third, sender);

    rollup.resume_preferred_batches().await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_latest_and_pending_match() -> anyhow::Result<()> {
    let nb_of_txs = 3;
    let nb_of_logs_per_tx: u32 = 2;

    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;
    let pending_block = latest_block_context(&rollup_and_client.client).await;
    let sender = rollup_and_client.client.address();

    let pending_filter = filter_for_tag(BlockNumberOrTag::Pending);
    let latest_filter = filter_for_tag(BlockNumberOrTag::Latest);

    let pending_logs = rollup_and_client.client.get_logs(&pending_filter).await;
    let latest_logs = rollup_and_client.client.get_logs(&latest_filter).await;

    let mut expected = Vec::new();
    for (tx_index, tx_hash) in tx_hashes.iter().enumerate() {
        for log_index_in_tx in 0..nb_of_logs_per_tx {
            let expected_meta = ExpectedLogMeta {
                address: rollup_and_client.contract_address,
                tx_hash: *tx_hash,
                tx_index: tx_index as u64,
                log_index: tx_index as u64 * nb_of_logs_per_tx as u64 + log_index_in_tx as u64,
                block_hash: pending_block.hash,
                block_number: pending_block.number,
                block_timestamp: Some(pending_block.timestamp),
            };
            expected.push(ExpectedSimpleLog {
                meta: expected_meta,
                topic1: U256::from(tx_index as u64),
                topic2: U256::from(log_index_in_tx as u64),
                data: U256::ZERO,
            });
        }
    }

    assert_expected_simple_logs(&pending_logs, &expected, sender);
    assert_eq!(pending_logs, latest_logs);
    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_pending_with_topic_filter() -> anyhow::Result<()> {
    let nb_of_txs = 5;
    let nb_of_logs_per_tx: u32 = 5;

    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;
    let pending_block = latest_block_context(&rollup_and_client.client).await;
    let sender = rollup_and_client.client.address();

    let topic: B256 = U256::from(3).into();
    let filter = filter_for_tag(BlockNumberOrTag::Pending).topic3(topic);

    let logs = rollup_and_client.client.get_logs(&filter).await;
    assert_eq!(logs.len() as u32, nb_of_txs);
    for (tx_index, tx_hash) in tx_hashes.iter().enumerate() {
        let expected_meta = ExpectedLogMeta {
            address: rollup_and_client.contract_address,
            tx_hash: *tx_hash,
            tx_index: tx_index as u64,
            log_index: (tx_index as u64) * nb_of_logs_per_tx as u64 + 3,
            block_hash: pending_block.hash,
            block_number: pending_block.number,
            block_timestamp: Some(pending_block.timestamp),
        };
        let log = &logs[tx_index];
        assert_simple_log(
            log,
            &expected_meta,
            sender,
            U256::from(tx_index as u64),
            U256::from(3),
            U256::ZERO,
        );
        assert!(filter.matches(log.inner.as_ref()));
    }
    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_default_range_matches_latest() -> anyhow::Result<()> {
    let nb_of_txs = 2;
    let nb_of_logs_per_tx: u32 = 3;

    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;
    let pending_block = latest_block_context(&rollup_and_client.client).await;
    let sender = rollup_and_client.client.address();

    let default_filter = Filter::new();
    let latest_filter = filter_for_tag(BlockNumberOrTag::Latest);

    let default_logs = rollup_and_client.client.get_logs(&default_filter).await;
    let latest_logs = rollup_and_client.client.get_logs(&latest_filter).await;

    let mut expected = Vec::new();
    for (tx_index, tx_hash) in tx_hashes.iter().enumerate() {
        for log_index_in_tx in 0..nb_of_logs_per_tx {
            let expected_meta = ExpectedLogMeta {
                address: rollup_and_client.contract_address,
                tx_hash: *tx_hash,
                tx_index: tx_index as u64,
                log_index: tx_index as u64 * nb_of_logs_per_tx as u64 + log_index_in_tx as u64,
                block_hash: pending_block.hash,
                block_number: pending_block.number,
                block_timestamp: Some(pending_block.timestamp),
            };
            expected.push(ExpectedSimpleLog {
                meta: expected_meta,
                topic1: U256::from(tx_index as u64),
                topic2: U256::from(log_index_in_tx as u64),
                data: U256::ZERO,
            });
        }
    }

    assert_expected_simple_logs(&default_logs, &expected, sender);
    assert_eq!(default_logs, latest_logs);

    // TODO: What is the point of doing that at the end of the test?
    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_from_greater_than_to_is_empty() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    rollup_and_client.produce_logs(1, 2, None).await;

    let filter = Filter::new()
        .from_block(BlockNumberOrTag::Latest)
        .to_block(BlockNumberOrTag::Earliest);
    let logs = rollup_and_client.client.get_logs(&filter).await;
    assert!(logs.is_empty());

    // TODO: What is the point of doing that at the end of the test?
    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_single_block_range() -> anyhow::Result<()> {
    let nb_of_logs_per_tx: u32 = 4;

    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let tx_hashes = rollup_and_client
        .produce_logs(2, nb_of_logs_per_tx, Some(1))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let block_number = rollup_and_client
        .client
        .alloy_receipt(tx_hashes[0])
        .await
        .unwrap()
        .block_number
        .expect("block number should be present");

    let filter = Filter::new()
        .from_block(block_number)
        .to_block(block_number);
    let logs = rollup_and_client.client.get_logs(&filter).await;

    let plans = [
        TxLogPlan {
            tx_hash: tx_hashes[0],
            log_count: nb_of_logs_per_tx as u64,
        },
        TxLogPlan {
            tx_hash: tx_hashes[1],
            log_count: nb_of_logs_per_tx as u64,
        },
    ];
    let meta_map = build_tx_log_meta(&rollup_and_client.client, &plans).await;
    let meta = meta_map
        .get(&tx_hashes[0])
        .expect("tx0 meta should be present");
    let sender = rollup_and_client.client.address();
    let mut expected = Vec::new();
    for log_index_in_tx in 0..nb_of_logs_per_tx {
        let expected_meta = ExpectedLogMeta {
            address: rollup_and_client.contract_address,
            tx_hash: tx_hashes[0],
            tx_index: meta.tx_index,
            log_index: meta.base_log_index + log_index_in_tx as u64,
            block_hash: meta.block_hash,
            block_number: meta.block_number,
            block_timestamp: Some(meta.block_timestamp),
        };
        expected.push(ExpectedSimpleLog {
            meta: expected_meta,
            topic1: U256::ZERO,
            topic2: U256::from(log_index_in_tx as u64),
            data: U256::ZERO,
        });
    }

    assert_expected_simple_logs(&logs, &expected, sender);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_address_filter_single() -> anyhow::Result<()> {
    let (test_rollup, client, contract_a, contract_b) = setup_two_contracts().await;

    test_rollup.pause_preferred_batches().await;
    let tx_a = client.alloy_emit_logs(contract_a, 1, 2).await;
    let _tx_b = client.alloy_emit_logs(contract_b, 2, 3).await;
    let pending_block = latest_block_context(&client).await;

    let filter = filter_for_tag(BlockNumberOrTag::Pending).address(contract_a);
    let logs = client.get_logs(&filter).await;

    assert_eq!(logs.len(), 2);
    for (idx, log) in logs.iter().enumerate() {
        let expected = ExpectedLogMeta {
            address: contract_a,
            tx_hash: tx_a,
            tx_index: 0,
            log_index: idx as u64,
            block_hash: pending_block.hash,
            block_number: pending_block.number,
            block_timestamp: Some(pending_block.timestamp),
        };
        assert_simple_log(
            log,
            &expected,
            client.address(),
            U256::from(1),
            U256::from(idx as u64),
            U256::ZERO,
        );
        assert!(filter.matches(log.inner.as_ref()));
    }

    test_rollup.resume_preferred_batches().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_address_filter_multiple() -> anyhow::Result<()> {
    let (test_rollup, client, contract_a, contract_b) = setup_two_contracts().await;

    test_rollup.pause_preferred_batches().await;
    let tx_a = client.alloy_emit_logs(contract_a, 1, 2).await;
    let tx_b = client.alloy_emit_logs(contract_b, 2, 3).await;
    let pending_block = latest_block_context(&client).await;

    let filter = filter_for_tag(BlockNumberOrTag::Pending).address(vec![contract_a, contract_b]);
    let logs = client.get_logs(&filter).await;

    assert_eq!(logs.len(), 5);
    for (idx, log) in logs.iter().enumerate() {
        let (address, tx_hash, tx_index, log_index_in_tx, topic1) = if idx < 2 {
            (contract_a, tx_a, 0u64, idx as u64, U256::from(1))
        } else {
            let log_index_in_tx = (idx - 2) as u64;
            (contract_b, tx_b, 1u64, log_index_in_tx, U256::from(2))
        };
        let expected = ExpectedLogMeta {
            address,
            tx_hash,
            tx_index,
            log_index: idx as u64,
            block_hash: pending_block.hash,
            block_number: pending_block.number,
            block_timestamp: Some(pending_block.timestamp),
        };
        assert_simple_log(
            log,
            &expected,
            client.address(),
            topic1,
            U256::from(log_index_in_tx),
            U256::ZERO,
        );
        assert!(filter.matches(log.inner.as_ref()));
    }

    test_rollup.resume_preferred_batches().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_topic_or_semantics() -> anyhow::Result<()> {
    let nb_of_txs = 2;
    let nb_of_logs_per_tx: u32 = 5;

    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;
    let pending_block = latest_block_context(&rollup_and_client.client).await;
    let sender = rollup_and_client.client.address();

    let topics = vec![U256::from(1).into(), U256::from(3).into()];
    let filter = filter_for_tag(BlockNumberOrTag::Pending).topic3(topics);
    let logs = rollup_and_client.client.get_logs(&filter).await;

    assert_eq!(logs.len() as u32, nb_of_txs * 2);
    let expected_log_indexes = [1u64, 3u64];
    let mut expected = Vec::new();
    for (tx_index, tx_hash) in tx_hashes.iter().enumerate() {
        for log_index_in_tx in expected_log_indexes {
            expected.push((tx_index as u64, *tx_hash, log_index_in_tx));
        }
    }
    for (log, (tx_index, tx_hash, log_index_in_tx)) in logs.iter().zip(expected) {
        let expected_meta = ExpectedLogMeta {
            address: rollup_and_client.contract_address,
            tx_hash,
            tx_index,
            log_index: tx_index * nb_of_logs_per_tx as u64 + log_index_in_tx,
            block_hash: pending_block.hash,
            block_number: pending_block.number,
            block_timestamp: Some(pending_block.timestamp),
        };
        assert_simple_log(
            log,
            &expected_meta,
            sender,
            U256::from(tx_index),
            U256::from(log_index_in_tx),
            U256::ZERO,
        );
        assert!(filter.matches(log.inner.as_ref()));
    }

    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_topic_and_with_wildcard() -> anyhow::Result<()> {
    let nb_of_logs_per_tx: u32 = 5;

    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    let tx_hash = rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 42, nb_of_logs_per_tx)
        .await;
    let pending_block = latest_block_context(&rollup_and_client.client).await;

    let sender = rollup_and_client.client.address();
    let filter = filter_for_tag(BlockNumberOrTag::Pending)
        .topic0(simple_log_topic0())
        .topic1(sender)
        .topic3(U256::from(4));
    let logs = rollup_and_client.client.get_logs(&filter).await;

    assert_eq!(logs.len(), 1);
    let expected = ExpectedLogMeta {
        address: rollup_and_client.contract_address,
        tx_hash,
        tx_index: 0,
        log_index: 4,
        block_hash: pending_block.hash,
        block_number: pending_block.number,
        block_timestamp: Some(pending_block.timestamp),
    };
    assert_simple_log(
        &logs[0],
        &expected,
        sender,
        U256::from(42),
        U256::from(4),
        U256::ZERO,
    );
    assert!(filter.matches(logs[0].inner.as_ref()));

    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_address_and_topic_intersection() -> anyhow::Result<()> {
    let (test_rollup, client, contract_a, contract_b) = setup_two_contracts().await;

    test_rollup.pause_preferred_batches().await;
    let tx_a = client.alloy_emit_logs(contract_a, 7, 3).await;
    let _tx_b = client.alloy_emit_logs(contract_b, 7, 3).await;
    let pending_block = latest_block_context(&client).await;

    let filter = filter_for_tag(BlockNumberOrTag::Pending)
        .address(contract_a)
        .topic3(U256::from(1));
    let logs = client.get_logs(&filter).await;

    assert_eq!(logs.len(), 1);
    let expected = ExpectedLogMeta {
        address: contract_a,
        tx_hash: tx_a,
        tx_index: 0,
        log_index: 1,
        block_hash: pending_block.hash,
        block_number: pending_block.number,
        block_timestamp: Some(pending_block.timestamp),
    };
    assert_simple_log(
        &logs[0],
        &expected,
        client.address(),
        U256::from(7),
        U256::from(1),
        U256::ZERO,
    );
    assert!(filter.matches(logs[0].inner.as_ref()));

    test_rollup.resume_preferred_batches().await;
    Ok(())
}

/// Tests that Safe/Finalized block tags exclude pending logs.
///
/// Safe and Finalized tags resolve to the latest sealed block number.
/// When querying logs with these tags, only logs from sealed blocks should
/// be returned - pending (unconfirmed) logs should be excluded.
///
/// Test flow:
/// 1. Emit 2 logs and seal them into a block
/// 2. Pause batching and emit 1 more pending log
/// 3. Query with Safe/Finalized range (0 to Safe/Finalized) - should return only sealed logs
/// 4. Query with Pending - should return all logs including pending
#[tokio::test(flavor = "multi_thread")]
async fn get_logs_safe_finalized_exclude_pending() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    // Emit 2 logs and seal them
    let sealed_tx = rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 1, 2)
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let sealed_hash = rollup_and_client
        .client
        .alloy_receipt(sealed_tx)
        .await
        .unwrap()
        .block_hash
        .expect("block hash should be present");

    // Pause batching and emit a pending log
    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;
    rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 2, 1)
        .await;

    // Query by block hash - baseline for sealed logs
    let sealed_logs = rollup_and_client
        .client
        .get_logs(&Filter::new().at_block_hash(sealed_hash))
        .await;
    assert_eq!(sealed_logs.len(), 2);

    // Verify sealed logs have correct metadata
    let meta_map = build_tx_log_meta(
        &rollup_and_client.client,
        &[TxLogPlan {
            tx_hash: sealed_tx,
            log_count: 2,
        }],
    )
    .await;
    let meta = meta_map
        .get(&sealed_tx)
        .expect("sealed tx meta should be present");
    let sender = rollup_and_client.client.address();
    for (idx, log) in sealed_logs.iter().enumerate() {
        let expected = ExpectedLogMeta {
            address: rollup_and_client.contract_address,
            tx_hash: sealed_tx,
            tx_index: meta.tx_index,
            log_index: meta.base_log_index + idx as u64,
            block_hash: meta.block_hash,
            block_number: meta.block_number,
            block_timestamp: Some(meta.block_timestamp),
        };
        assert_simple_log(
            log,
            &expected,
            sender,
            U256::from(1),
            U256::from(idx as u64),
            U256::ZERO,
        );
    }

    // Query from genesis to Safe - should include sealed logs, exclude pending
    let safe_filter = Filter::new().from_block(0).to_block(BlockNumberOrTag::Safe);
    let safe_logs = rollup_and_client.client.get_logs(&safe_filter).await;
    assert_eq!(
        safe_logs, sealed_logs,
        "Safe range should match sealed logs"
    );

    // Query from genesis to Finalized - should include sealed logs, exclude pending
    let finalized_filter = Filter::new()
        .from_block(0)
        .to_block(BlockNumberOrTag::Finalized);
    let finalized_logs = rollup_and_client.client.get_logs(&finalized_filter).await;
    assert_eq!(
        finalized_logs, sealed_logs,
        "Finalized range should match sealed logs"
    );

    // Query with Pending - should include both sealed and pending logs
    let pending_filter = Filter::new()
        .from_block(0)
        .to_block(BlockNumberOrTag::Pending);
    let pending_logs = rollup_and_client.client.get_logs(&pending_filter).await;
    assert!(pending_logs.len() > sealed_logs.len());
    assert_eq!(
        pending_logs.len(),
        3,
        "Pending should include sealed (2) + pending (1) logs"
    );

    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_ordered_and_not_removed() -> anyhow::Result<()> {
    let nb_of_txs = 4;
    let nb_of_logs_per_tx: u32 = 3;

    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(1))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let logs = rollup_and_client
        .client
        .get_logs(&new_filter_for_all_logs())
        .await;
    let sender = rollup_and_client.client.address();
    let plans = simple_log_plans_from_hashes(&tx_hashes, nb_of_logs_per_tx as u64, U256::ZERO);
    let expected = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;
    assert_expected_simple_logs(&logs, &expected, sender);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_time_executed_ms_per_tx() -> anyhow::Result<()> {
    let (test_rollup, evm_client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract_address = evm_client.alloy_deploy_contract().await;
    test_rollup.wait_for_next_blocks(1).await;

    let start_block = evm_client
        .eth_get_block_by_number(Some(BlockNumberOrTag::Latest.to_string()))
        .await
        .number();

    let tx1 = evm_client.alloy_emit_logs(contract_address, 0, 3).await;
    let tx2 = evm_client.alloy_emit_logs(contract_address, 1, 2).await;
    test_rollup.wait_for_next_blocks(1).await;

    let filter = Filter::new()
        .from_block(start_block)
        .to_block(BlockNumberOrTag::Latest);
    let logs = evm_client.get_logs_with_timestamp(&filter).await;
    assert!(!logs.is_empty());

    let sender = evm_client.address();
    let plans = [
        SimpleLogPlan {
            tx_hash: tx1,
            topic1: U256::ZERO,
            log_count: 3,
            data: U256::ZERO,
        },
        SimpleLogPlan {
            tx_hash: tx2,
            topic1: U256::from(1),
            log_count: 2,
            data: U256::ZERO,
        },
    ];
    let expected = expected_simple_logs_from_plans(&evm_client, contract_address, &plans).await;

    let mut times_by_tx = HashMap::new();
    assert_eq!(logs.len(), expected.len());
    for (log_with_time, expected_log) in logs.iter().zip(expected.iter()) {
        assert_simple_log(
            &log_with_time.log,
            &expected_log.meta,
            sender,
            expected_log.topic1,
            expected_log.topic2,
            expected_log.data,
        );
        let tx_hash = log_with_time
            .log
            .transaction_hash
            .expect("transaction hash should be present");
        let entry = times_by_tx
            .entry(tx_hash)
            .or_insert(log_with_time.time_executed_ms);
        assert_eq!(*entry, log_with_time.time_executed_ms);
        assert!(log_with_time.time_executed_ms > 0);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_emitted_fields_match_event() -> anyhow::Result<()> {
    let nb_of_logs_per_tx: u32 = 3;

    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let sender = rollup_and_client.client.address();
    let topic1_base = U256::from(7);
    let topic2_base = U256::from(19);
    let tx_hash = rollup_and_client
        .client
        .alloy_emit_configurable_logs(
            rollup_and_client.contract_address,
            topic1_base,
            topic2_base,
            nb_of_logs_per_tx,
        )
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let receipt = rollup_and_client
        .client
        .alloy_receipt(tx_hash)
        .await
        .unwrap();
    let block_hash = receipt.block_hash.expect("block hash should be present");
    let meta_map = build_tx_log_meta(
        &rollup_and_client.client,
        &[TxLogPlan {
            tx_hash,
            log_count: nb_of_logs_per_tx as u64,
        }],
    )
    .await;
    let meta = meta_map
        .get(&tx_hash)
        .expect("configurable logs tx meta should be present");

    let filter = Filter::new()
        .at_block_hash(block_hash)
        .address(rollup_and_client.contract_address)
        .topic0(simple_log_topic0());
    let logs = rollup_and_client.client.get_logs(&filter).await;
    assert_eq!(logs.len() as u32, nb_of_logs_per_tx);

    for (idx, log) in logs.iter().enumerate() {
        let idx_u256 = U256::from(idx as u64);
        let expected_topic1 = topic1_base + idx_u256;
        let expected_topic2 = topic2_base + idx_u256;
        let expected = ExpectedLogMeta {
            address: rollup_and_client.contract_address,
            tx_hash,
            tx_index: meta.tx_index,
            log_index: meta.base_log_index + idx as u64,
            block_hash: meta.block_hash,
            block_number: meta.block_number,
            block_timestamp: Some(meta.block_timestamp),
        };
        assert_simple_log(
            log,
            &expected,
            sender,
            expected_topic1,
            expected_topic2,
            idx_u256,
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_full_topic_log() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let t0 = U256::from_be_slice(&[0x11u8; 32]);
    let t1 = U256::from_be_slice(&[0x22u8; 32]);
    let t2 = U256::from_be_slice(&[0x33u8; 32]);
    let data = U256::from_be_slice(&[0x44u8; 32]);

    let tx_hash = rollup_and_client
        .client
        .alloy_emit_full_topic_log(rollup_and_client.contract_address, t0, t1, t2, data)
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let receipt = rollup_and_client
        .client
        .alloy_receipt(tx_hash)
        .await
        .unwrap();
    let block_hash = receipt.block_hash.expect("block hash should be present");

    let filter = Filter::new()
        .at_block_hash(block_hash)
        .address(rollup_and_client.contract_address)
        .topic0(full_topic_log_topic0());
    let logs = rollup_and_client.client.get_logs(&filter).await;
    assert_eq!(logs.len(), 1);

    let meta_map = build_tx_log_meta(
        &rollup_and_client.client,
        &[TxLogPlan {
            tx_hash,
            log_count: 1,
        }],
    )
    .await;
    let meta = meta_map
        .get(&tx_hash)
        .expect("full topic tx meta should be present");
    let log = &logs[0];
    let expected = ExpectedLogMeta {
        address: rollup_and_client.contract_address,
        tx_hash,
        tx_index: meta.tx_index,
        log_index: meta.base_log_index,
        block_hash: meta.block_hash,
        block_number: meta.block_number,
        block_timestamp: Some(meta.block_timestamp),
    };
    assert_full_topic_log(log, &expected, t0, t1, t2, data);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_data_only_log() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let v1 = U256::from_be_slice(&[0x55u8; 32]);
    let v2 = U256::from_be_slice(&[0x66u8; 32]);
    let tx_hash = rollup_and_client
        .client
        .alloy_emit_data_only_log(rollup_and_client.contract_address, v1, v2)
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let receipt = rollup_and_client
        .client
        .alloy_receipt(tx_hash)
        .await
        .unwrap();
    let block_hash = receipt.block_hash.expect("block hash should be present");

    let filter = Filter::new()
        .at_block_hash(block_hash)
        .address(rollup_and_client.contract_address)
        .topic0(data_only_log_topic0());
    let logs = rollup_and_client.client.get_logs(&filter).await;
    assert_eq!(logs.len(), 1);

    let meta_map = build_tx_log_meta(
        &rollup_and_client.client,
        &[TxLogPlan {
            tx_hash,
            log_count: 1,
        }],
    )
    .await;
    let meta = meta_map
        .get(&tx_hash)
        .expect("data-only tx meta should be present");
    let log = &logs[0];
    let expected = ExpectedLogMeta {
        address: rollup_and_client.contract_address,
        tx_hash,
        tx_index: meta.tx_index,
        log_index: meta.base_log_index,
        block_hash: meta.block_hash,
        block_number: meta.block_number,
        block_timestamp: Some(meta.block_timestamp),
    };
    assert_data_only_log(log, &expected, v1, v2);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_indexed_only_log_data_empty() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let value = U256::from(42);
    let tx_hash = rollup_and_client
        .client
        .alloy_emit_indexed_only_log(rollup_and_client.contract_address, value)
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let receipt = rollup_and_client
        .client
        .alloy_receipt(tx_hash)
        .await
        .unwrap();
    let block_hash = receipt.block_hash.expect("block hash should be present");

    let filter = Filter::new()
        .at_block_hash(block_hash)
        .address(rollup_and_client.contract_address)
        .topic0(indexed_only_log_topic0())
        .topic1(B256::from(value));
    let logs = rollup_and_client.client.get_logs(&filter).await;
    assert_eq!(logs.len(), 1);

    let meta_map = build_tx_log_meta(
        &rollup_and_client.client,
        &[TxLogPlan {
            tx_hash,
            log_count: 1,
        }],
    )
    .await;
    let meta = meta_map
        .get(&tx_hash)
        .expect("indexed-only tx meta should be present");
    let log = &logs[0];
    let expected = ExpectedLogMeta {
        address: rollup_and_client.contract_address,
        tx_hash,
        tx_index: meta.tx_index,
        log_index: meta.base_log_index,
        block_hash: meta.block_hash,
        block_number: meta.block_number,
        block_timestamp: Some(meta.block_timestamp),
    };
    assert_indexed_only_log(log, &expected, value);

    let filter_json = serde_json::json!({
        "blockHash": format!("{:#x}", block_hash),
        "address": format!("{:#x}", rollup_and_client.contract_address),
        "topics": [
            format!("{:#x}", indexed_only_log_topic0()),
            format!("{:#x}", B256::from(value)),
        ],
    });
    let json_logs = get_logs_raw_json(&rollup_and_client.client, filter_json).await;
    assert_eq!(json_logs.len(), 1);
    let data = json_logs[0]
        .get("data")
        .and_then(|v| v.as_str())
        .expect("data should be a string");
    assert_eq!(data, "0x");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_topic0_filters_event_signature() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    let simple_tx = rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 1, 2)
        .await;
    let data_tx = rollup_and_client
        .client
        .alloy_emit_data_only_log(
            rollup_and_client.contract_address,
            U256::from(1),
            U256::from(2),
        )
        .await;
    let pending_block = latest_block_context(&rollup_and_client.client).await;
    let sender = rollup_and_client.client.address();

    let simple_filter = filter_for_tag(BlockNumberOrTag::Pending)
        .address(rollup_and_client.contract_address)
        .topic0(simple_log_topic0());
    let simple_logs = rollup_and_client.client.get_logs(&simple_filter).await;
    assert_eq!(simple_logs.len(), 2);
    for (idx, log) in simple_logs.iter().enumerate() {
        let expected = ExpectedLogMeta {
            address: rollup_and_client.contract_address,
            tx_hash: simple_tx,
            tx_index: 0,
            log_index: idx as u64,
            block_hash: pending_block.hash,
            block_number: pending_block.number,
            block_timestamp: Some(pending_block.timestamp),
        };
        assert_simple_log(
            log,
            &expected,
            sender,
            U256::from(1),
            U256::from(idx as u64),
            U256::ZERO,
        );
    }

    let data_filter = filter_for_tag(BlockNumberOrTag::Pending)
        .address(rollup_and_client.contract_address)
        .topic0(data_only_log_topic0());
    let data_logs = rollup_and_client.client.get_logs(&data_filter).await;
    assert_eq!(data_logs.len(), 1);
    let expected = ExpectedLogMeta {
        address: rollup_and_client.contract_address,
        tx_hash: data_tx,
        tx_index: 1,
        log_index: 2,
        block_hash: pending_block.hash,
        block_number: pending_block.number,
        block_timestamp: Some(pending_block.timestamp),
    };
    assert_data_only_log(&data_logs[0], &expected, U256::from(1), U256::from(2));

    // TODO: What is the point of doing that at the end of the test?
    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_earliest_tag() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let tx_hashes = rollup_and_client.produce_logs(3, 2, Some(1)).await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;
    let last_tx = *tx_hashes.last().expect("expected log tx hash");
    let last_receipt = rollup_and_client
        .client
        .alloy_receipt(last_tx)
        .await
        .unwrap();
    let max_block = last_receipt
        .block_number
        .expect("block number should be present");

    let earliest_filter = Filter::new()
        .from_block(BlockNumberOrTag::Earliest)
        .to_block(max_block);
    let zero_filter = Filter::new().from_block(0).to_block(max_block);

    let earliest_logs = rollup_and_client.client.get_logs(&earliest_filter).await;
    let zero_logs = rollup_and_client.client.get_logs(&zero_filter).await;

    let sender = rollup_and_client.client.address();
    let plans = simple_log_plans_from_hashes(&tx_hashes, 2, U256::ZERO);
    let expected = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;

    assert_expected_simple_logs(&earliest_logs, &expected, sender);
    assert_eq!(earliest_logs, zero_logs);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_schema_correctness() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let tx_hashes = rollup_and_client.produce_logs(2, 3, Some(1)).await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;
    let last_tx = *tx_hashes.last().expect("expected log tx hash");
    let last_receipt = rollup_and_client
        .client
        .alloy_receipt(last_tx)
        .await
        .unwrap();
    let max_block = last_receipt
        .block_number
        .expect("block number should be present");

    let filter = serde_json::json!({
        "fromBlock": "0x0",
        "toBlock": format!("0x{max_block:x}"),
        "address": format!("{:#x}", rollup_and_client.contract_address),
    });
    let logs = get_logs_raw_json(&rollup_and_client.client, filter).await;
    let sender = rollup_and_client.client.address();
    let plans = simple_log_plans_from_hashes(&tx_hashes, 3, U256::ZERO);
    let expected = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;

    assert_eq!(logs.len(), expected.len());
    for (log, expected_log) in logs.iter().zip(expected.iter()) {
        assert_log_json_schema(log);
        assert_log_json_matches_expected_simple_log(log, expected_log, sender);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "alloy-rpc-eth does not support using BlockId in range. Our own implementation will require considerable effort"]
async fn get_logs_eip1898_blockhash_object() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let tx_hash = rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 7, 2)
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let receipt = rollup_and_client
        .client
        .alloy_receipt(tx_hash)
        .await
        .unwrap();
    let block_hash = receipt.block_hash.expect("block hash should be present");

    let filter = serde_json::json!({
        "fromBlock": {
            "blockHash": format!("{:#x}", block_hash),
            "requireCanonical": true,
        },
        "toBlock": {
            "blockHash": format!("{:#x}", block_hash),
            "requireCanonical": true,
        },
        "address": format!("{:#x}", rollup_and_client.contract_address),
    });
    let eip_logs = get_logs_raw_typed(&rollup_and_client.client, filter).await;
    let sender = rollup_and_client.client.address();
    let plans = [SimpleLogPlan {
        tx_hash,
        topic1: U256::from(7),
        log_count: 2,
        data: U256::ZERO,
    }];
    let expected_logs = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;
    assert_expected_simple_logs(&eip_logs, &expected_logs, sender);

    let expected = rollup_and_client
        .client
        .get_logs(
            &Filter::new()
                .at_block_hash(block_hash)
                .address(rollup_and_client.contract_address),
        )
        .await;
    assert_eq!(eip_logs, expected);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_empty_result() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    rollup_and_client.produce_logs(1, 2, None).await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let empty_address = Address::from([0x11; 20]);
    let filter = Filter::new()
        .from_block(0)
        .to_block(BlockNumberOrTag::Latest)
        .address(empty_address);
    let logs = rollup_and_client.client.get_logs(&filter).await;
    assert!(logs.is_empty());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_empty_topics_matches_all() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let tx_hashes = rollup_and_client.produce_logs(2, 2, Some(1)).await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;
    let last_tx = *tx_hashes.last().expect("expected log tx hash");
    let last_receipt = rollup_and_client
        .client
        .alloy_receipt(last_tx)
        .await
        .unwrap();
    let max_block = last_receipt
        .block_number
        .expect("block number should be present");

    let filter_with_empty_topics = serde_json::json!({
        "fromBlock": "0x0",
        "toBlock": format!("0x{max_block:x}"),
        "address": format!("{:#x}", rollup_and_client.contract_address),
        "topics": [],
    });
    let logs_with_empty_topics =
        get_logs_raw_typed(&rollup_and_client.client, filter_with_empty_topics).await;

    let filter = Filter::new()
        .from_block(0)
        .to_block(max_block)
        .address(rollup_and_client.contract_address);
    let logs = rollup_and_client.client.get_logs(&filter).await;

    let sender = rollup_and_client.client.address();
    let plans = simple_log_plans_from_hashes(&tx_hashes, 2, U256::ZERO);
    let expected = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;
    assert_expected_simple_logs(&logs_with_empty_topics, &expected, sender);
    assert_eq!(logs_with_empty_topics, logs);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_topic0_only_matches_any_indexed() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let tx_hash = rollup_and_client
        .client
        .alloy_emit_configurable_logs(
            rollup_and_client.contract_address,
            U256::from(10),
            U256::from(20),
            3,
        )
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let receipt = rollup_and_client
        .client
        .alloy_receipt(tx_hash)
        .await
        .unwrap();
    let block_hash = receipt.block_hash.expect("block hash should be present");

    let filter = Filter::new()
        .at_block_hash(block_hash)
        .address(rollup_and_client.contract_address)
        .topic0(simple_log_topic0());
    let logs = rollup_and_client.client.get_logs(&filter).await;

    let meta_map = build_tx_log_meta(
        &rollup_and_client.client,
        &[TxLogPlan {
            tx_hash,
            log_count: 3,
        }],
    )
    .await;
    let meta = meta_map
        .get(&tx_hash)
        .expect("configurable logs tx meta should be present");
    let sender = rollup_and_client.client.address();
    assert_eq!(logs.len(), 3);
    for (idx, log) in logs.iter().enumerate() {
        let expected_meta = ExpectedLogMeta {
            address: rollup_and_client.contract_address,
            tx_hash,
            tx_index: meta.tx_index,
            log_index: meta.base_log_index + idx as u64,
            block_hash: meta.block_hash,
            block_number: meta.block_number,
            block_timestamp: Some(meta.block_timestamp),
        };
        let idx_u256 = U256::from(idx as u64);
        assert_simple_log(
            log,
            &expected_meta,
            sender,
            U256::from(10) + idx_u256,
            U256::from(20) + idx_u256,
            idx_u256,
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_topic1_without_topic0() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let sender = rollup_and_client.client.address();
    let simple_tx = rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 5, 3)
        .await;
    let data_tx = rollup_and_client
        .client
        .alloy_emit_data_only_log(
            rollup_and_client.contract_address,
            U256::from(1),
            U256::from(2),
        )
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;
    let simple_receipt = rollup_and_client
        .client
        .alloy_receipt(simple_tx)
        .await
        .unwrap();
    let data_receipt = rollup_and_client
        .client
        .alloy_receipt(data_tx)
        .await
        .unwrap();
    let max_block = simple_receipt
        .block_number
        .expect("block number should be present")
        .max(
            data_receipt
                .block_number
                .expect("block number should be present"),
        );

    let filter = Filter::new()
        .from_block(0)
        .to_block(max_block)
        .address(rollup_and_client.contract_address)
        .topic1(sender);
    let logs = rollup_and_client.client.get_logs(&filter).await;

    let meta_map = build_tx_log_meta(
        &rollup_and_client.client,
        &[
            TxLogPlan {
                tx_hash: simple_tx,
                log_count: 3,
            },
            TxLogPlan {
                tx_hash: data_tx,
                log_count: 1,
            },
        ],
    )
    .await;
    let meta = meta_map
        .get(&simple_tx)
        .expect("simple tx meta should be present");
    let mut expected = Vec::new();
    for log_index_in_tx in 0..3u64 {
        let expected_meta = ExpectedLogMeta {
            address: rollup_and_client.contract_address,
            tx_hash: simple_tx,
            tx_index: meta.tx_index,
            log_index: meta.base_log_index + log_index_in_tx,
            block_hash: meta.block_hash,
            block_number: meta.block_number,
            block_timestamp: Some(meta.block_timestamp),
        };
        expected.push(ExpectedSimpleLog {
            meta: expected_meta,
            topic1: U256::from(5),
            topic2: U256::from(log_index_in_tx),
            data: U256::ZERO,
        });
    }

    assert_expected_simple_logs(&logs, &expected, sender);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_topic0_or_semantics_multiple() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let simple_tx = rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 1, 2)
        .await;
    let data_tx = rollup_and_client
        .client
        .alloy_emit_data_only_log(
            rollup_and_client.contract_address,
            U256::from(9),
            U256::from(10),
        )
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;
    let simple_receipt = rollup_and_client
        .client
        .alloy_receipt(simple_tx)
        .await
        .unwrap();
    let data_receipt = rollup_and_client
        .client
        .alloy_receipt(data_tx)
        .await
        .unwrap();
    let max_block = simple_receipt
        .block_number
        .expect("block number should be present")
        .max(
            data_receipt
                .block_number
                .expect("block number should be present"),
        );

    let filter = Filter::new()
        .from_block(0)
        .to_block(max_block)
        .address(rollup_and_client.contract_address)
        .topic0(vec![simple_log_topic0(), data_only_log_topic0()]);
    let logs = rollup_and_client.client.get_logs(&filter).await;

    let meta_map = build_tx_log_meta(
        &rollup_and_client.client,
        &[
            TxLogPlan {
                tx_hash: simple_tx,
                log_count: 2,
            },
            TxLogPlan {
                tx_hash: data_tx,
                log_count: 1,
            },
        ],
    )
    .await;
    let simple_meta = meta_map
        .get(&simple_tx)
        .expect("simple tx meta should be present");
    let data_meta = meta_map
        .get(&data_tx)
        .expect("data-only tx meta should be present");

    let mut expected = Vec::new();
    for log_index_in_tx in 0..2u64 {
        expected.push(ExpectedLogEntry {
            meta: ExpectedLogMeta {
                address: rollup_and_client.contract_address,
                tx_hash: simple_tx,
                tx_index: simple_meta.tx_index,
                log_index: simple_meta.base_log_index + log_index_in_tx,
                block_hash: simple_meta.block_hash,
                block_number: simple_meta.block_number,
                block_timestamp: Some(simple_meta.block_timestamp),
            },
            kind: ExpectedLogKind::Simple {
                topic1: U256::from(1),
                topic2: U256::from(log_index_in_tx),
                data: U256::ZERO,
            },
        });
    }
    expected.push(ExpectedLogEntry {
        meta: ExpectedLogMeta {
            address: rollup_and_client.contract_address,
            tx_hash: data_tx,
            tx_index: data_meta.tx_index,
            log_index: data_meta.base_log_index,
            block_hash: data_meta.block_hash,
            block_number: data_meta.block_number,
            block_timestamp: Some(data_meta.block_timestamp),
        },
        kind: ExpectedLogKind::DataOnly {
            v1: U256::from(9),
            v2: U256::from(10),
        },
    });
    expected.sort_by_key(|entry| {
        (
            entry.meta.block_number,
            entry.meta.tx_index,
            entry.meta.log_index,
        )
    });

    assert_expected_log_entries(&logs, &expected, rollup_and_client.client.address());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_topic0_and_topic1_or_semantics() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let sender = rollup_and_client.client.address();
    let full_topic_value = U256::from(777);
    let simple_tx = rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 7, 1)
        .await;
    let full_tx = rollup_and_client
        .client
        .alloy_emit_full_topic_log(
            rollup_and_client.contract_address,
            full_topic_value,
            U256::from(1),
            U256::from(2),
            U256::from(3),
        )
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;
    let simple_receipt = rollup_and_client
        .client
        .alloy_receipt(simple_tx)
        .await
        .unwrap();
    let full_receipt = rollup_and_client
        .client
        .alloy_receipt(full_tx)
        .await
        .unwrap();
    let max_block = simple_receipt
        .block_number
        .expect("block number should be present")
        .max(
            full_receipt
                .block_number
                .expect("block number should be present"),
        );

    let filter = Filter::new()
        .from_block(0)
        .to_block(max_block)
        .address(rollup_and_client.contract_address)
        .topic0(vec![simple_log_topic0(), full_topic_log_topic0()])
        .topic1(vec![address_to_topic(sender), B256::from(full_topic_value)]);
    let logs = rollup_and_client.client.get_logs(&filter).await;

    let meta_map = build_tx_log_meta(
        &rollup_and_client.client,
        &[
            TxLogPlan {
                tx_hash: simple_tx,
                log_count: 1,
            },
            TxLogPlan {
                tx_hash: full_tx,
                log_count: 1,
            },
        ],
    )
    .await;
    let simple_meta = meta_map
        .get(&simple_tx)
        .expect("simple tx meta should be present");
    let full_meta = meta_map
        .get(&full_tx)
        .expect("full topic tx meta should be present");

    let mut expected = vec![
        ExpectedLogEntry {
            meta: ExpectedLogMeta {
                address: rollup_and_client.contract_address,
                tx_hash: simple_tx,
                tx_index: simple_meta.tx_index,
                log_index: simple_meta.base_log_index,
                block_hash: simple_meta.block_hash,
                block_number: simple_meta.block_number,
                block_timestamp: Some(simple_meta.block_timestamp),
            },
            kind: ExpectedLogKind::Simple {
                topic1: U256::from(7),
                topic2: U256::ZERO,
                data: U256::ZERO,
            },
        },
        ExpectedLogEntry {
            meta: ExpectedLogMeta {
                address: rollup_and_client.contract_address,
                tx_hash: full_tx,
                tx_index: full_meta.tx_index,
                log_index: full_meta.base_log_index,
                block_hash: full_meta.block_hash,
                block_number: full_meta.block_number,
                block_timestamp: Some(full_meta.block_timestamp),
            },
            kind: ExpectedLogKind::FullTopic {
                t0: full_topic_value,
                t1: U256::from(1),
                t2: U256::from(2),
                data: U256::from(3),
            },
        },
    ];
    expected.sort_by_key(|entry| {
        (
            entry.meta.block_number,
            entry.meta.tx_index,
            entry.meta.log_index,
        )
    });

    assert_expected_log_entries(&logs, &expected, sender);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_trailing_null_topics_ignored() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let tx_hashes = rollup_and_client.produce_logs(2, 2, Some(1)).await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;
    let last_tx = *tx_hashes.last().expect("expected log tx hash");
    let last_receipt = rollup_and_client
        .client
        .alloy_receipt(last_tx)
        .await
        .unwrap();
    let max_block = last_receipt
        .block_number
        .expect("block number should be present");

    let filter_with_nulls = serde_json::json!({
        "fromBlock": "0x0",
        "toBlock": format!("0x{max_block:x}"),
        "address": format!("{:#x}", rollup_and_client.contract_address),
        "topics": [
            format!("{:#x}", simple_log_topic0()),
            null,
            null
        ],
    });
    let logs_with_nulls = get_logs_raw_typed(&rollup_and_client.client, filter_with_nulls).await;

    let filter = Filter::new()
        .from_block(0)
        .to_block(max_block)
        .address(rollup_and_client.contract_address)
        .topic0(simple_log_topic0());
    let logs = rollup_and_client.client.get_logs(&filter).await;

    let sender = rollup_and_client.client.address();
    let plans = simple_log_plans_from_hashes(&tx_hashes, 2, U256::ZERO);
    let expected = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;
    assert_expected_simple_logs(&logs_with_nulls, &expected, sender);
    assert_eq!(logs_with_nulls, logs);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs() {
    let nb_of_txs = 10;
    let nb_of_logs_per_tx = 5;

    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    // Make sure all the txs are in the same blcok.
    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;

    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let tx_hash = tx_hashes[0];
    let rec = rollup_and_client
        .client
        .alloy_receipt(tx_hash)
        .await
        .unwrap();
    let block_hash = rec.block_hash.unwrap();

    let sender = rollup_and_client.client.address();
    let plans = simple_log_plans_from_hashes(&tx_hashes, nb_of_logs_per_tx as u64, U256::ZERO);
    let expected_all = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;
    {
        let filter = Filter::new().at_block_hash(block_hash);
        let logs = rollup_and_client.client.get_logs(&filter).await;
        assert_expected_simple_logs(&logs, &expected_all, sender);
    }

    // topic3 seolects one log from each tx
    {
        let topic: B256 = U256::from(3).into();
        let filter = Filter::new().at_block_hash(block_hash).topic3(topic);

        let logs = rollup_and_client.client.get_logs(&filter).await;
        let expected_topic3 = filter_expected_by_topic2(&expected_all, U256::from(3));
        assert_expected_simple_logs(&logs, &expected_topic3, sender);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_range() {
    let nb_of_txs = 10;
    let nb_of_logs_per_tx = 5;

    let rollup_and_client = RollupAndClient::new_with_default_limits().await;

    let start_block = rollup_and_client
        .client
        .eth_get_block_by_number(Some(BlockNumberOrTag::Latest.to_string()))
        .await
        .number();

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let sender = rollup_and_client.client.address();
    let plans = simple_log_plans_from_hashes(&tx_hashes, nb_of_logs_per_tx as u64, U256::ZERO);
    let expected_all = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;

    // Check logs from all txs.
    {
        let filter = Filter::new()
            .from_block(start_block)
            .to_block(BlockNumberOrTag::Latest);

        let logs = rollup_and_client.client.get_logs(&filter).await;
        assert_expected_simple_logs(&logs, &expected_all, sender);
    }

    // topic3 seolects one log from each tx
    {
        let topic: B256 = U256::from(3).into();
        let filter = Filter::new()
            .from_block(start_block)
            .to_block(BlockNumberOrTag::Latest)
            .topic3(topic);

        let logs = rollup_and_client.client.get_logs(&filter).await;
        let expected_topic3 = filter_expected_by_topic2(&expected_all, U256::from(3));
        assert_expected_simple_logs(&logs, &expected_topic3, sender);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_range_limit() {
    let max_log_limit = 93;
    let nb_of_txs = 20;
    let nb_of_logs_per_tx: u32 = 5;

    let rollup_and_client =
        RollupAndClient::new(max_log_limit, EVM_EXTENSION.response_size_limit).await;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let logs = rollup_and_client.client.get_logs_allow_error().await;
    assert!(logs.is_err());
    assert!(logs.unwrap_err().to_string().contains("Response size exceeds limit. Use eth_getLogsWithCursor or reduce the number of logs requested"));
}

fn simple_log_topic0() -> B256 {
    keccak256(b"SimpleLog(address,uint256,uint256,uint256)")
}

fn full_topic_log_topic0() -> B256 {
    keccak256(b"FullTopicLog(uint256,uint256,uint256,uint256)")
}

fn data_only_log_topic0() -> B256 {
    keccak256(b"DataOnlyLog(uint256,uint256)")
}

fn indexed_only_log_topic0() -> B256 {
    keccak256(b"IndexedOnlyLog(uint256)")
}

fn address_to_topic(address: Address) -> B256 {
    let mut bytes = [0u8; 32];
    bytes[12..].copy_from_slice(address.as_slice());
    B256::from(bytes)
}

fn decode_two_u256(data: &[u8]) -> (U256, U256) {
    assert_eq!(data.len(), 64);
    let (first, second) = data.split_at(32);
    (U256::from_be_slice(first), U256::from_be_slice(second))
}

struct BlockContext {
    hash: B256,
    number: u64,
    timestamp: u64,
}

#[derive(Clone, Copy)]
struct ExpectedLogMeta {
    address: Address,
    tx_hash: alloy_primitives::TxHash,
    tx_index: u64,
    log_index: u64,
    block_hash: B256,
    block_number: u64,
    block_timestamp: Option<u64>,
}

#[derive(Clone, Copy)]
struct TxLogPlan {
    tx_hash: TxHash,
    log_count: u64,
}

struct TxLogMeta {
    block_hash: B256,
    block_number: u64,
    tx_index: u64,
    base_log_index: u64,
    block_timestamp: u64,
}

#[derive(Clone)]
struct SimpleLogPlan {
    tx_hash: TxHash,
    topic1: U256,
    log_count: u64,
    data: U256,
}

#[derive(Clone)]
struct ExpectedSimpleLog {
    meta: ExpectedLogMeta,
    topic1: U256,
    topic2: U256,
    data: U256,
}

enum ExpectedLogKind {
    Simple {
        topic1: U256,
        topic2: U256,
        data: U256,
    },
    DataOnly {
        v1: U256,
        v2: U256,
    },
    FullTopic {
        t0: U256,
        t1: U256,
        t2: U256,
        data: U256,
    },
}

struct ExpectedLogEntry {
    meta: ExpectedLogMeta,
    kind: ExpectedLogKind,
}

async fn block_context_by_number(client: &SimpleStorageClient, number: u64) -> BlockContext {
    let block = client
        .eth_get_block_by_number(Some(format!("0x{number:x}")))
        .await;
    BlockContext {
        hash: block.header.hash,
        number: block.number(),
        timestamp: block.header.timestamp,
    }
}

async fn latest_block_context(client: &SimpleStorageClient) -> BlockContext {
    let block = client
        .eth_get_block_by_number(Some(BlockNumberOrTag::Latest.to_string()))
        .await;
    BlockContext {
        hash: block.header.hash,
        number: block.number(),
        timestamp: block.header.timestamp,
    }
}

async fn build_tx_log_meta(
    client: &SimpleStorageClient,
    plans: &[TxLogPlan],
) -> HashMap<TxHash, TxLogMeta> {
    struct PlanReceipt {
        plan: TxLogPlan,
        block_number: u64,
        block_hash: B256,
        tx_index: u64,
    }

    let mut by_block: HashMap<u64, Vec<PlanReceipt>> = HashMap::new();
    for plan in plans {
        let receipt = client
            .alloy_receipt(plan.tx_hash)
            .await
            .expect("receipt should be present for test tx");
        let block_number = receipt
            .block_number
            .expect("block number should be present");
        let block_hash = receipt.block_hash.expect("block hash should be present");
        let tx_index = receipt
            .transaction_index
            .expect("transaction index should be present");
        by_block.entry(block_number).or_default().push(PlanReceipt {
            plan: *plan,
            block_number,
            block_hash,
            tx_index,
        });
    }

    let mut meta_map = HashMap::new();
    for (block_number, mut entries) in by_block {
        entries.sort_by_key(|entry| entry.tx_index);
        let block = block_context_by_number(client, block_number).await;
        let mut base_log_index = 0u64;
        for entry in entries {
            assert_eq!(block.hash, entry.block_hash);
            meta_map.insert(
                entry.plan.tx_hash,
                TxLogMeta {
                    block_hash: entry.block_hash,
                    block_number: entry.block_number,
                    tx_index: entry.tx_index,
                    base_log_index,
                    block_timestamp: block.timestamp,
                },
            );
            base_log_index += entry.plan.log_count;
        }
    }

    meta_map
}

async fn expected_simple_logs_from_plans(
    client: &SimpleStorageClient,
    contract_address: Address,
    plans: &[SimpleLogPlan],
) -> Vec<ExpectedSimpleLog> {
    let tx_plans: Vec<TxLogPlan> = plans
        .iter()
        .map(|plan| TxLogPlan {
            tx_hash: plan.tx_hash,
            log_count: plan.log_count,
        })
        .collect();
    let meta_map = build_tx_log_meta(client, &tx_plans).await;
    let mut expected = Vec::new();
    for plan in plans {
        let meta = meta_map
            .get(&plan.tx_hash)
            .expect("tx meta should be present");
        for log_index_in_tx in 0..plan.log_count {
            let expected_meta = ExpectedLogMeta {
                address: contract_address,
                tx_hash: plan.tx_hash,
                tx_index: meta.tx_index,
                log_index: meta.base_log_index + log_index_in_tx,
                block_hash: meta.block_hash,
                block_number: meta.block_number,
                block_timestamp: Some(meta.block_timestamp),
            };
            expected.push(ExpectedSimpleLog {
                meta: expected_meta,
                topic1: plan.topic1,
                topic2: U256::from(log_index_in_tx),
                data: plan.data,
            });
        }
    }
    expected.sort_by_key(|entry| {
        (
            entry.meta.block_number,
            entry.meta.tx_index,
            entry.meta.log_index,
        )
    });
    expected
}

fn simple_log_plans_from_hashes(
    tx_hashes: &[TxHash],
    log_count: u64,
    data: U256,
) -> Vec<SimpleLogPlan> {
    tx_hashes
        .iter()
        .enumerate()
        .map(|(idx, tx_hash)| SimpleLogPlan {
            tx_hash: *tx_hash,
            topic1: U256::from(idx as u64),
            log_count,
            data,
        })
        .collect()
}

fn filter_expected_by_topic2(
    expected: &[ExpectedSimpleLog],
    topic2: U256,
) -> Vec<ExpectedSimpleLog> {
    expected
        .iter()
        .filter(|log| log.topic2 == topic2)
        .cloned()
        .collect()
}

async fn expected_simple_logs_for_rollup(
    rollup_and_client: &RollupAndClient,
    plans: &[SimpleLogPlan],
) -> Vec<ExpectedSimpleLog> {
    expected_simple_logs_from_plans(
        &rollup_and_client.client,
        rollup_and_client.contract_address,
        plans,
    )
    .await
}

fn assert_expected_simple_logs(logs: &[Log], expected: &[ExpectedSimpleLog], sender: Address) {
    assert_eq!(logs.len(), expected.len());
    for (log, expected_log) in logs.iter().zip(expected.iter()) {
        assert_simple_log(
            log,
            &expected_log.meta,
            sender,
            expected_log.topic1,
            expected_log.topic2,
            expected_log.data,
        );
    }
}

fn assert_expected_log_entries(logs: &[Log], expected: &[ExpectedLogEntry], sender: Address) {
    assert_eq!(logs.len(), expected.len());
    for (log, expected_log) in logs.iter().zip(expected.iter()) {
        match &expected_log.kind {
            ExpectedLogKind::Simple {
                topic1,
                topic2,
                data,
            } => {
                assert_simple_log(log, &expected_log.meta, sender, *topic1, *topic2, *data);
            }
            ExpectedLogKind::DataOnly { v1, v2 } => {
                assert_data_only_log(log, &expected_log.meta, *v1, *v2);
            }
            ExpectedLogKind::FullTopic { t0, t1, t2, data } => {
                assert_full_topic_log(log, &expected_log.meta, *t0, *t1, *t2, *data);
            }
        }
    }
}

fn assert_log_meta(log: &Log, expected: &ExpectedLogMeta) {
    assert_eq!(log.address(), expected.address);
    assert_eq!(log.transaction_hash, Some(expected.tx_hash));
    assert_eq!(log.transaction_index, Some(expected.tx_index));
    assert_eq!(log.log_index, Some(expected.log_index));
    assert_eq!(log.block_hash, Some(expected.block_hash));
    assert_eq!(log.block_number, Some(expected.block_number));
    if let Some(expected_timestamp) = expected.block_timestamp {
        assert_eq!(log.block_timestamp, Some(expected_timestamp));
    }
    assert!(!log.removed);
}

fn assert_simple_log(
    log: &Log,
    expected: &ExpectedLogMeta,
    sender: Address,
    topic1: U256,
    topic2: U256,
    data: U256,
) {
    assert_log_meta(log, expected);
    let expected_topics = vec![
        simple_log_topic0(),
        address_to_topic(sender),
        B256::from(topic1),
        B256::from(topic2),
    ];
    assert_eq!(log.inner.topics(), expected_topics.as_slice());
    assert_eq!(U256::from_be_slice(log.inner.data.data.as_ref()), data);
}

fn assert_full_topic_log(
    log: &Log,
    expected: &ExpectedLogMeta,
    t0: U256,
    t1: U256,
    t2: U256,
    data: U256,
) {
    assert_log_meta(log, expected);
    let expected_topics = vec![
        full_topic_log_topic0(),
        B256::from(t0),
        B256::from(t1),
        B256::from(t2),
    ];
    assert_eq!(log.inner.topics(), expected_topics.as_slice());
    assert_eq!(U256::from_be_slice(log.inner.data.data.as_ref()), data);
}

fn assert_data_only_log(log: &Log, expected: &ExpectedLogMeta, v1: U256, v2: U256) {
    assert_log_meta(log, expected);
    let expected_topics = vec![data_only_log_topic0()];
    assert_eq!(log.inner.topics(), expected_topics.as_slice());
    let data_bytes = log.inner.data.data.as_ref();
    let (decoded_v1, decoded_v2) = decode_two_u256(data_bytes);
    assert_eq!(decoded_v1, v1);
    assert_eq!(decoded_v2, v2);
}

fn assert_indexed_only_log(log: &Log, expected: &ExpectedLogMeta, value: U256) {
    assert_log_meta(log, expected);
    let expected_topics = vec![indexed_only_log_topic0(), B256::from(value)];
    assert_eq!(log.inner.topics(), expected_topics.as_slice());
    assert!(log.inner.data.data.is_empty());
}

fn u64_to_quantity_hex(value: u64) -> String {
    format!("0x{value:x}")
}

fn u256_to_data_hex(value: U256) -> String {
    format!("0x{value:064x}")
}

fn assert_log_json_matches_expected_simple_log(
    log: &Value,
    expected: &ExpectedSimpleLog,
    sender: Address,
) {
    let obj = log.as_object().expect("log should be a JSON object");
    let expected_address = format!("{:#x}", expected.meta.address);
    assert_eq!(
        obj.get("address")
            .and_then(|v| v.as_str())
            .expect("address should be a string"),
        expected_address
    );
    let expected_block_hash = format!("{:#x}", expected.meta.block_hash);
    assert_eq!(
        obj.get("blockHash")
            .and_then(|v| v.as_str())
            .expect("blockHash should be a string"),
        expected_block_hash
    );
    assert_eq!(
        obj.get("blockNumber")
            .and_then(|v| v.as_str())
            .expect("blockNumber should be a string"),
        u64_to_quantity_hex(expected.meta.block_number)
    );
    let expected_tx_hash = format!("{:#x}", expected.meta.tx_hash);
    assert_eq!(
        obj.get("transactionHash")
            .and_then(|v| v.as_str())
            .expect("transactionHash should be a string"),
        expected_tx_hash
    );
    assert_eq!(
        obj.get("transactionIndex")
            .and_then(|v| v.as_str())
            .expect("transactionIndex should be a string"),
        u64_to_quantity_hex(expected.meta.tx_index)
    );
    assert_eq!(
        obj.get("logIndex")
            .and_then(|v| v.as_str())
            .expect("logIndex should be a string"),
        u64_to_quantity_hex(expected.meta.log_index)
    );
    assert_eq!(
        obj.get("data")
            .and_then(|v| v.as_str())
            .expect("data should be a string"),
        u256_to_data_hex(expected.data)
    );

    let topics = obj
        .get("topics")
        .and_then(|v| v.as_array())
        .expect("topics should be an array");
    let expected_topics = [
        format!("{:#x}", simple_log_topic0()),
        format!("{:#x}", address_to_topic(sender)),
        format!("{:#x}", B256::from(expected.topic1)),
        format!("{:#x}", B256::from(expected.topic2)),
    ];
    assert_eq!(topics.len(), expected_topics.len());
    for (topic, expected_topic) in topics.iter().zip(expected_topics.iter()) {
        let actual = topic.as_str().expect("topic should be a string");
        assert_eq!(actual, expected_topic);
    }

    let removed = obj
        .get("removed")
        .and_then(|v| v.as_bool())
        .expect("removed should be a bool");
    assert!(!removed);
}

fn assert_hex_fixed(value: &Value, expected_len: usize) {
    let hex = value
        .as_str()
        .expect("expected hex string for fixed-size field");
    assert!(hex.starts_with("0x"));
    assert_eq!(hex.len(), expected_len);
}

fn assert_hex_quantity(value: &Value) -> u64 {
    let hex = value
        .as_str()
        .expect("expected hex string for quantity field");
    assert!(hex.starts_with("0x"));
    let digits = &hex[2..];
    assert!(!digits.is_empty());
    u64::from_str_radix(digits, 16).expect("valid hex quantity")
}

fn assert_hex_data(value: &Value) {
    let hex = value.as_str().expect("expected hex string for data field");
    assert!(hex.starts_with("0x"));
    let digits = &hex[2..];
    assert!(digits.len() % 2 == 0);
}

fn assert_log_json_schema(log: &Value) {
    let obj = log.as_object().expect("log should be a JSON object");
    assert_hex_fixed(obj.get("address").expect("address should be present"), 42);
    assert_hex_fixed(
        obj.get("blockHash").expect("blockHash should be present"),
        66,
    );
    assert_hex_quantity(
        obj.get("blockNumber")
            .expect("blockNumber should be present"),
    );
    assert_hex_fixed(
        obj.get("transactionHash")
            .expect("transactionHash should be present"),
        66,
    );
    assert_hex_quantity(
        obj.get("transactionIndex")
            .expect("transactionIndex should be present"),
    );
    assert_hex_quantity(obj.get("logIndex").expect("logIndex should be present"));
    assert_hex_data(obj.get("data").expect("data should be present"));

    let topics = obj.get("topics").expect("topics should be present");
    let topics = topics.as_array().expect("topics should be an array");
    assert!(topics.len() <= 4);
    assert!(!topics.is_empty());
    for topic in topics {
        assert_hex_fixed(topic, 66);
    }

    let removed = obj.get("removed").expect("removed should be present");
    assert!(removed.as_bool().is_some());
}

async fn get_logs_raw_typed(client: &SimpleStorageClient, filter: Value) -> Vec<Log> {
    client
        .ws
        .request("eth_getLogs", rpc_params![filter])
        .await
        .unwrap()
}

async fn get_logs_raw_json(client: &SimpleStorageClient, filter: Value) -> Vec<Value> {
    client
        .ws
        .request("eth_getLogs", rpc_params![filter])
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_with_cursor_and_filter() {
    let max_log_limit = 9;
    let nb_of_txs = 20;
    let nb_of_logs_per_tx: u32 = 7;

    let rollup_and_client =
        RollupAndClient::new(max_log_limit, EVM_EXTENSION.response_size_limit).await;
    let start_tx = rollup_and_client.get_tx_count().await as u32;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let plans = simple_log_plans_from_hashes(&tx_hashes, nb_of_logs_per_tx as u64, U256::ZERO);
    let expected = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;
    let sender = rollup_and_client.client.address();

    let mut nb_of_logs_received = 0;
    let mut collected = Vec::new();
    let mut logs_with_cursor = rollup_and_client
        .get_logs_with_cursor_and_filter(&serde_json::json!({
                "fromBlock": "0x0",
                "toBlock": "latest",
        }))
        .await;

    nb_of_logs_received += logs_with_cursor.logs.len();
    collected.extend(logs_with_cursor.logs.iter().map(|log| log.log.clone()));

    let mut nb_of_logs_until_prev_cursor = start_tx * nb_of_logs_per_tx;
    loop {
        let packed_cursor = match logs_with_cursor.cursor {
            Some(packed) => packed,
            None => {
                let total = (nb_of_txs * nb_of_logs_per_tx) as usize;
                assert_eq!(logs_with_cursor.logs.len(), total % max_log_limit);
                break;
            }
        };

        let cursor = Cursor::unpack(&packed_cursor).unwrap();
        let nb_of_logs_according_to_cursor =
            nb_of_logs_according_to_cursor(cursor, nb_of_logs_per_tx);

        // Every cursor gives max_log_limit logs.
        assert_eq!(
            nb_of_logs_according_to_cursor - nb_of_logs_until_prev_cursor,
            max_log_limit as u32
        );

        nb_of_logs_until_prev_cursor = nb_of_logs_according_to_cursor;

        logs_with_cursor = rollup_and_client
            .get_logs_with_cursor_and_filter(&serde_json::json!({
                "fromBlock": "0x0",
                "toBlock": "latest",
                "cursor": packed_cursor,
            }))
            .await;
        nb_of_logs_received += logs_with_cursor.logs.len();
        collected.extend(logs_with_cursor.logs.iter().map(|log| log.log.clone()));
    }

    assert_eq!(nb_of_logs_received as u32, nb_of_txs * nb_of_logs_per_tx);
    assert_expected_simple_logs(&collected, &expected, sender);
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_with_cursor() {
    let max_log_limit = 9;
    let nb_of_txs = 20;
    let nb_of_logs_per_tx: u32 = 7;

    let rollup_and_client =
        RollupAndClient::new(max_log_limit, EVM_EXTENSION.response_size_limit).await;
    let start_tx = rollup_and_client.get_tx_count().await as u32;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let plans = simple_log_plans_from_hashes(&tx_hashes, nb_of_logs_per_tx as u64, U256::ZERO);
    let expected = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;
    let sender = rollup_and_client.client.address();

    let mut nb_of_logs_received = 0;
    let mut collected = Vec::new();
    let mut logs_with_cursor = rollup_and_client.get_logs_with_cursor(None).await;

    nb_of_logs_received += logs_with_cursor.logs.len();
    collected.extend(logs_with_cursor.logs.iter().map(|log| log.log.clone()));

    let mut nb_of_logs_until_prev_cursor = start_tx * nb_of_logs_per_tx;
    loop {
        let packed_cursor = match logs_with_cursor.cursor {
            Some(packed) => packed,
            None => {
                let total = (nb_of_txs * nb_of_logs_per_tx) as usize;
                assert_eq!(logs_with_cursor.logs.len(), total % max_log_limit);
                break;
            }
        };

        let cursor = Cursor::unpack(&packed_cursor).unwrap();
        let nb_of_logs_according_to_cursor =
            nb_of_logs_according_to_cursor(cursor, nb_of_logs_per_tx);

        // Every cursor gives max_log_limit logs.
        assert_eq!(
            nb_of_logs_according_to_cursor - nb_of_logs_until_prev_cursor,
            max_log_limit as u32
        );

        nb_of_logs_until_prev_cursor = nb_of_logs_according_to_cursor;

        logs_with_cursor = rollup_and_client.get_logs_with_cursor(Some(cursor)).await;
        nb_of_logs_received += logs_with_cursor.logs.len();
        collected.extend(logs_with_cursor.logs.iter().map(|log| log.log.clone()));
    }

    assert_eq!(nb_of_logs_received as u32, nb_of_txs * nb_of_logs_per_tx);
    assert_expected_simple_logs(&collected, &expected, sender);
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_at_max_response_size() {
    let nb_of_txs = 3;
    let nb_of_logs_per_tx: u32 = 40;

    // Set a low limit on response sizes to force pagination.
    let rollup_and_client = RollupAndClient::new(EVM_EXTENSION.max_log_limit, 1_000).await;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let plans = simple_log_plans_from_hashes(&tx_hashes, nb_of_logs_per_tx as u64, U256::ZERO);
    let expected = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;
    let sender = rollup_and_client.client.address();

    let mut nb_of_logs_received = 0;
    let mut collected = Vec::new();
    let mut logs_with_cursor = rollup_and_client.get_logs_with_cursor(None).await;

    nb_of_logs_received += logs_with_cursor.logs.len();
    collected.extend(logs_with_cursor.logs.iter().map(|log| log.log.clone()));

    let mut iters = 0;
    while let Some(packed_cursor) = logs_with_cursor.cursor {
        iters += 1;

        let cursor = Cursor::unpack(&packed_cursor).unwrap();
        logs_with_cursor = rollup_and_client.get_logs_with_cursor(Some(cursor)).await;
        nb_of_logs_received += logs_with_cursor.logs.len();
        collected.extend(logs_with_cursor.logs.iter().map(|log| log.log.clone()));
    }

    assert_eq!(nb_of_logs_received as u32, nb_of_txs * nb_of_logs_per_tx);
    assert!(iters > 1, "We should have reached the response size limit and been forced to paginate. This test might need adjusting, or pagination is broken.");
    assert_expected_simple_logs(&collected, &expected, sender);
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_at_max_response_size_without_cursor_throws_error() {
    let nb_of_txs = 3;
    let nb_of_logs_per_tx: u32 = 50;
    let rollup_and_client = RollupAndClient::new(100, EVM_EXTENSION.response_size_limit).await;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let err = rollup_and_client
        .client
        .get_logs_allow_error()
        .await
        .unwrap_err();

    assert!(err.to_string().contains("Response size exceeds limit. Use eth_getLogsWithCursor or reduce the number of logs requested"), "Unexpected error: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_resumed_from_the_middle_of_tx_have_correct_indices() {
    let max_log_limit = 1;
    let nb_of_txs = 1;
    let nb_of_logs_per_tx: u32 = 2;

    let rollup_and_client =
        RollupAndClient::new(max_log_limit, EVM_EXTENSION.response_size_limit).await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let plans = simple_log_plans_from_hashes(&tx_hashes, nb_of_logs_per_tx as u64, U256::ZERO);
    let expected = expected_simple_logs_for_rollup(&rollup_and_client, &plans).await;
    let sender = rollup_and_client.client.address();

    let LogsWithMaybeCursor { mut logs, cursor } =
        rollup_and_client.get_logs_with_cursor(None).await;
    let cursor = cursor.map(|c| Cursor::unpack(&c).unwrap()).unwrap();
    assert_eq!(logs.len(), 1);
    let log = logs.pop().unwrap();
    let expected_log = &expected[0];
    assert_simple_log(
        &log.log,
        &expected_log.meta,
        sender,
        expected_log.topic1,
        expected_log.topic2,
        expected_log.data,
    );
    assert_eq!(log.log.transaction_index, Some(0));
    assert_eq!(log.log.log_index, Some(0));
    assert_eq!(cursor.tx_index_absolute, 1);
    assert_eq!(cursor.log_index_in_tx, 1);

    let LogsWithMaybeCursor { mut logs, .. } =
        rollup_and_client.get_logs_with_cursor(Some(cursor)).await;
    assert_eq!(logs.len(), 1);
    let log = logs.pop().unwrap();
    let expected_log = &expected[1];
    assert_simple_log(
        &log.log,
        &expected_log.meta,
        sender,
        expected_log.topic1,
        expected_log.topic2,
        expected_log.data,
    );
    assert_eq!(log.log.transaction_index, Some(0));
    assert_eq!(log.log.log_index, Some(1));
}

fn nb_of_logs_according_to_cursor(cursor: Cursor, nb_of_logs_per_tx: u32) -> u32 {
    (cursor.tx_index_absolute as u32) * nb_of_logs_per_tx + cursor.log_index_in_tx
}

fn filter_for_tag(tag: BlockNumberOrTag) -> Filter {
    Filter::new().from_block(tag).to_block(tag)
}

fn new_filter_for_all_logs() -> Filter {
    Filter::new()
        .from_block(0)
        .to_block(BlockNumberOrTag::Latest)
}

async fn setup_two_contracts() -> (
    TestRollup<MockDemoRollup<Native>>,
    SimpleStorageClient,
    Address,
    Address,
) {
    let (test_rollup, evm_client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract_a = evm_client.alloy_deploy_contract().await;
    let contract_b = evm_client.alloy_deploy_contract().await;
    test_rollup.wait_for_next_blocks(1).await;
    (test_rollup, evm_client, contract_a, contract_b)
}

struct RollupAndClient {
    test_rollup: TestRollup<MockDemoRollup<Native>>,
    client: SimpleStorageClient,
    contract_address: alloy_primitives::Address,
}

impl RollupAndClient {
    async fn new_with_default_limits() -> RollupAndClient {
        RollupAndClient::new(
            EVM_EXTENSION.max_log_limit,
            EVM_EXTENSION.response_size_limit,
        )
        .await
    }

    async fn new(max_log_limit: usize, response_size_limit: usize) -> RollupAndClient {
        let ext = SeqConfigExtension {
            max_log_limit,
            response_size_limit,
        };

        let (test_rollup, evm_client, _) = setup_with_simple_storage(0, ext).await;
        let contract_address = evm_client.alloy_deploy_contract().await;
        test_rollup.wait_for_next_blocks(1).await;

        RollupAndClient {
            test_rollup,
            client: evm_client,
            contract_address,
        }
    }

    async fn produce_logs(
        &self,
        nb_of_txs: u32,
        nb_of_logs_per_tx: u32,
        block_interval: Option<u32>,
    ) -> Vec<alloy_primitives::TxHash> {
        let mut tx_hashes = Vec::with_capacity(nb_of_txs as usize);
        for i in 0..nb_of_txs {
            let hash = self
                .client
                .alloy_emit_logs(self.contract_address, i, nb_of_logs_per_tx)
                .await;
            tx_hashes.push(hash);
            if let Some(block_interval) = block_interval {
                if i % block_interval == 0 {
                    self.test_rollup.wait_for_next_blocks(1).await;
                }
            }
        }

        tx_hashes
    }

    async fn get_tx_count(&self) -> u64 {
        self.client
            .eth_get_transaction_count(self.client.address())
            .await
    }

    async fn get_logs_with_cursor(&self, cursor: Option<Cursor>) -> LogsWithMaybeCursor {
        let filter_with_cursor = FilterWithCursor {
            cursor: cursor.map(|c| c.pack()),
            filter: new_filter_for_all_logs(),
        };

        self.client.get_logs_with_cursor(&filter_with_cursor).await
    }

    async fn get_logs_with_cursor_and_filter(
        &self,
        cursor_and_filter: &impl serde::Serialize,
    ) -> LogsWithMaybeCursor {
        self.client
            .get_logs_with_cursor_and_filter(cursor_and_filter)
            .await
    }
}
