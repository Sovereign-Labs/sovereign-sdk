// This is fo
#![allow(deprecated)]
use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::setup_with_simple_storage;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_provider::Provider;
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter};
use sov_demo_rollup::MockDemoRollup;
use sov_eth_client::SimpleStorageClient;
use sov_ethereum::Cursor;
use sov_evm_test_utils::SimpleStorage;
use sov_modules_api::execution_mode::Native;
use sov_rpc_eth_types::FilterWithCursor;
use sov_rpc_eth_types::LogsWithMaybeCursor;
use sov_sequencer::SeqConfigExtension;
use sov_test_utils::test_rollup::TestRollup;
use std::collections::HashMap;

#[tokio::test(flavor = "multi_thread")]
async fn get_log_from_pending_block() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_client(rollup.http_addr);
    let contract = SimpleStorage::deploy(client.clone()).await?;
    rollup.pause_preferred_batches().await;

    let _first_tx = contract.emitLogs(U256::ZERO, U256::from(2)).send().await?;
    let pending_filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending);
    let logs_after_first = client.get_logs(&pending_filter).await?;
    assert_eq!(logs_after_first.len(), 2);

    let first_hash = client
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .expect("latest block should be present")
        .header
        .hash;

    let _second_tx = contract
        .emitLogs(U256::from(1), U256::from(1))
        .send()
        .await?;
    let _third_tx = contract
        .emitLogs(U256::from(2), U256::from(1))
        .send()
        .await?;

    let logs_after_all = client.get_logs(&pending_filter).await?;
    let hash_filter = Filter::new().at_block_hash(first_hash);
    let logs_by_hash = client.get_logs(&hash_filter).await?;

    assert!(!logs_by_hash.is_empty());
    assert_eq!(logs_by_hash, logs_after_first);
    assert!(logs_after_all.starts_with(&logs_by_hash));

    let log = &logs_by_hash[0];
    let pending_hash = log
        .block_hash
        .expect("pending logs should include a synthetic block hash");
    assert_ne!(pending_hash, B256::ZERO);
    assert_eq!(pending_hash, first_hash);
    assert_ne!(
        log.block_timestamp
            .expect("block timestamp should be present"),
        0
    );

    rollup.resume_preferred_batches().await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_latest_and_pending_match() -> anyhow::Result<()> {
    let nb_of_txs = 3;
    let nb_of_logs_per_tx: u32 = 2;

    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;

    let pending_filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending);
    let latest_filter = Filter::new()
        .from_block(BlockNumberOrTag::Latest)
        .to_block(BlockNumberOrTag::Latest);

    let pending_logs = rollup_and_client.client.get_logs(&pending_filter).await;
    let latest_logs = rollup_and_client.client.get_logs(&latest_filter).await;

    assert!(!pending_logs.is_empty());
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

    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;

    let topic: B256 = U256::from(3).into();
    let filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending)
        .topic3(topic);

    let logs = rollup_and_client.client.get_logs(&filter).await;
    assert_eq!(logs.len() as u32, nb_of_txs);
    for log in logs {
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

    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;

    let default_filter = Filter::new();
    let latest_filter = Filter::new()
        .from_block(BlockNumberOrTag::Latest)
        .to_block(BlockNumberOrTag::Latest);

    let default_logs = rollup_and_client.client.get_logs(&default_filter).await;
    let latest_logs = rollup_and_client.client.get_logs(&latest_filter).await;

    assert!(!default_logs.is_empty());
    assert_eq!(default_logs, latest_logs);

    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_from_greater_than_to_is_empty() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

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

    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_single_block_range() -> anyhow::Result<()> {
    let nb_of_logs_per_tx: u32 = 4;

    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

    let tx_hashes = rollup_and_client
        .produce_logs(2, nb_of_logs_per_tx, Some(1))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let receipt = rollup_and_client
        .client
        .alloy_receipt(tx_hashes[0])
        .await
        .unwrap();
    let block_number = receipt.block_number.unwrap();

    let filter = Filter::new()
        .from_block(block_number)
        .to_block(block_number);
    let logs = rollup_and_client.client.get_logs(&filter).await;

    assert_eq!(logs.len() as u32, nb_of_logs_per_tx);
    for log in logs {
        assert_eq!(log.block_number, Some(block_number));
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_address_filter_single() -> anyhow::Result<()> {
    let (test_rollup, client, contract_a, contract_b) = setup_two_contracts().await;

    test_rollup.pause_preferred_batches().await;
    client.alloy_emit_logs(contract_a, 1, 2).await;
    client.alloy_emit_logs(contract_b, 2, 3).await;

    let filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending)
        .address(contract_a);
    let logs = client.get_logs(&filter).await;

    assert_eq!(logs.len(), 2);
    for log in logs {
        assert_eq!(log.address(), contract_a);
        assert!(filter.matches(log.inner.as_ref()));
    }

    test_rollup.resume_preferred_batches().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_address_filter_multiple() -> anyhow::Result<()> {
    let (test_rollup, client, contract_a, contract_b) = setup_two_contracts().await;

    test_rollup.pause_preferred_batches().await;
    client.alloy_emit_logs(contract_a, 1, 2).await;
    client.alloy_emit_logs(contract_b, 2, 3).await;

    let filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending)
        .address(vec![contract_a, contract_b]);
    let logs = client.get_logs(&filter).await;

    assert_eq!(logs.len(), 5);
    for log in logs {
        assert!(filter.matches(log.inner.as_ref()));
    }

    test_rollup.resume_preferred_batches().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_topic_or_semantics() -> anyhow::Result<()> {
    let nb_of_txs = 2;
    let nb_of_logs_per_tx: u32 = 5;

    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;

    let topics = vec![U256::from(1).into(), U256::from(3).into()];
    let filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending)
        .topic3(topics);
    let logs = rollup_and_client.client.get_logs(&filter).await;

    assert_eq!(logs.len() as u32, nb_of_txs * 2);
    for log in logs {
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

    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 42, nb_of_logs_per_tx)
        .await;

    let filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending)
        .topic2(U256::from(42))
        .topic3(U256::from(4));
    let logs = rollup_and_client.client.get_logs(&filter).await;

    assert_eq!(logs.len(), 1);
    for log in logs {
        assert!(filter.matches(log.inner.as_ref()));
    }

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
    client.alloy_emit_logs(contract_a, 7, 3).await;
    client.alloy_emit_logs(contract_b, 7, 3).await;

    let filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending)
        .address(contract_a)
        .topic3(U256::from(1));
    let logs = client.get_logs(&filter).await;

    assert_eq!(logs.len(), 1);
    for log in logs {
        assert_eq!(log.address(), contract_a);
        assert!(filter.matches(log.inner.as_ref()));
    }

    test_rollup.resume_preferred_batches().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_safe_finalized_exclude_pending() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

    let sealed_tx = rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 1, 2)
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let sealed_receipt = rollup_and_client
        .client
        .alloy_receipt(sealed_tx)
        .await
        .unwrap();
    let sealed_hash = sealed_receipt.block_hash.unwrap();

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;
    rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 2, 1)
        .await;

    let sealed_logs = rollup_and_client
        .client
        .get_logs(&Filter::new().at_block_hash(sealed_hash))
        .await;

    let safe_filter = Filter::new()
        .from_block(BlockNumberOrTag::Safe)
        .to_block(BlockNumberOrTag::Safe);
    let safe_logs = rollup_and_client.client.get_logs(&safe_filter).await;
    assert_eq!(safe_logs, sealed_logs);

    let finalized_filter = Filter::new()
        .from_block(BlockNumberOrTag::Finalized)
        .to_block(BlockNumberOrTag::Finalized);
    let finalized_logs = rollup_and_client.client.get_logs(&finalized_filter).await;
    assert_eq!(finalized_logs, sealed_logs);

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

    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(1))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let logs = rollup_and_client
        .client
        .get_logs(&new_filter_for_all_logs())
        .await;
    assert!(!logs.is_empty());
    assert_logs_ordered_and_not_removed(&logs);
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

    let _tx1 = evm_client.alloy_emit_logs(contract_address, 0, 3).await;
    let _tx2 = evm_client.alloy_emit_logs(contract_address, 1, 2).await;
    test_rollup.wait_for_next_blocks(1).await;

    let filter = Filter::new()
        .from_block(start_block)
        .to_block(BlockNumberOrTag::Latest);
    let logs = evm_client.get_logs_with_timestamp(&filter).await;
    assert!(!logs.is_empty());

    let mut times_by_tx = HashMap::new();
    for log in logs {
        let tx_hash = log
            .log
            .transaction_hash
            .expect("transaction hash should be present");
        let entry = times_by_tx.entry(tx_hash).or_insert(log.time_executed_ms);
        assert_eq!(*entry, log.time_executed_ms);
        assert!(log.time_executed_ms > 0);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_emitted_fields_match_event() -> anyhow::Result<()> {
    let nb_of_logs_per_tx: u32 = 3;

    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

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
    let block_number = receipt
        .block_number
        .expect("block number should be present");

    let filter = Filter::new()
        .at_block_hash(block_hash)
        .address(rollup_and_client.contract_address)
        .topic0(simple_log_topic0());
    let logs = rollup_and_client.client.get_logs(&filter).await;
    assert_eq!(logs.len() as u32, nb_of_logs_per_tx);

    let topic0 = simple_log_topic0();
    let sender_topic = address_to_topic(sender);
    for (idx, log) in logs.iter().enumerate() {
        let idx_u256 = U256::from(idx as u64);
        let expected_topic1 = topic1_base + idx_u256;
        let expected_topic2 = topic2_base + idx_u256;
        let topics = log.inner.topics();
        assert_eq!(topics.len(), 4);
        assert_eq!(topics[0], topic0);
        assert_eq!(topics[1], sender_topic);
        assert_eq!(topics[2], B256::from(expected_topic1));
        assert_eq!(topics[3], B256::from(expected_topic2));

        assert_eq!(log.address(), rollup_and_client.contract_address);
        assert_eq!(log.transaction_hash, Some(tx_hash));
        assert_eq!(log.transaction_index, Some(0));
        assert_eq!(log.log_index, Some(idx as u64));
        assert_eq!(log.block_number, Some(block_number));
        assert_eq!(log.block_hash, Some(block_hash));

        let data = log.inner.data.data.as_ref();
        assert_eq!(data.len(), 32);
        assert_eq!(U256::from_be_slice(data), idx_u256);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_full_topic_log() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

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

    let log = &logs[0];
    let topics = log.inner.topics();
    assert_eq!(topics.len(), 4);
    assert_eq!(topics[0], full_topic_log_topic0());
    assert_eq!(topics[1], B256::from(t0));
    assert_eq!(topics[2], B256::from(t1));
    assert_eq!(topics[3], B256::from(t2));
    assert_eq!(log.address(), rollup_and_client.contract_address);
    assert_eq!(log.transaction_hash, Some(tx_hash));

    let data_bytes = log.inner.data.data.as_ref();
    assert_eq!(data_bytes.len(), 32);
    assert_eq!(U256::from_be_slice(data_bytes), data);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_data_only_log() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

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

    let log = &logs[0];
    let topics = log.inner.topics();
    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0], data_only_log_topic0());
    assert_eq!(log.address(), rollup_and_client.contract_address);
    assert_eq!(log.transaction_hash, Some(tx_hash));

    let data_bytes = log.inner.data.data.as_ref();
    assert_eq!(data_bytes.len(), 64);
    let (decoded_v1, decoded_v2) = decode_two_u256(data_bytes);
    assert_eq!(decoded_v1, v1);
    assert_eq!(decoded_v2, v2);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_topic0_filters_event_signature() -> anyhow::Result<()> {
    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    let _simple_tx = rollup_and_client
        .client
        .alloy_emit_logs(rollup_and_client.contract_address, 1, 2)
        .await;
    let _data_tx = rollup_and_client
        .client
        .alloy_emit_data_only_log(
            rollup_and_client.contract_address,
            U256::from(1),
            U256::from(2),
        )
        .await;

    let simple_filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending)
        .address(rollup_and_client.contract_address)
        .topic0(simple_log_topic0());
    let simple_logs = rollup_and_client.client.get_logs(&simple_filter).await;
    assert_eq!(simple_logs.len(), 2);
    for log in simple_logs {
        let topics = log.inner.topics();
        assert_eq!(topics.len(), 4);
        assert_eq!(topics[0], simple_log_topic0());
    }

    let data_filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending)
        .address(rollup_and_client.contract_address)
        .topic0(data_only_log_topic0());
    let data_logs = rollup_and_client.client.get_logs(&data_filter).await;
    assert_eq!(data_logs.len(), 1);
    let topics = data_logs[0].inner.topics();
    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0], data_only_log_topic0());

    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs() {
    let nb_of_txs = 10;
    let nb_of_logs_per_tx = 5;

    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

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

    {
        let filter = Filter::new().at_block_hash(block_hash);
        let logs = rollup_and_client.client.get_logs(&filter).await;
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

        let logs = rollup_and_client.client.get_logs(&filter).await;
        check_logs(&filter, logs, nb_of_txs);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_range() {
    let nb_of_txs = 10;
    let nb_of_logs_per_tx = 5;

    let rollup_and_client = RollupAndClient::new(
        EVM_EXTENSION.max_log_limit,
        EVM_EXTENSION.response_size_limit,
    )
    .await;

    let start_block = rollup_and_client
        .client
        .eth_get_block_by_number(Some(BlockNumberOrTag::Latest.to_string()))
        .await
        .number();

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    // Check logs from all txs.
    {
        let filter = Filter::new()
            .from_block(start_block)
            .to_block(BlockNumberOrTag::Latest);

        let logs = rollup_and_client.client.get_logs(&filter).await;
        check_logs(&filter, logs, nb_of_txs * nb_of_logs_per_tx);
    }

    // topic3 seolects one log from each tx
    {
        let topic: B256 = U256::from(3).into();
        let filter = Filter::new()
            .from_block(start_block)
            .to_block(BlockNumberOrTag::Latest)
            .topic3(topic);

        let logs = rollup_and_client.client.get_logs(&filter).await;
        check_logs(&filter, logs, nb_of_txs);
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

fn check_logs(filter: &Filter, logs: Vec<alloy_rpc_types_eth::Log>, expected_nb_of_logs: u32) {
    assert_eq!(logs.len() as u32, expected_nb_of_logs);
    for log in logs {
        assert!(filter.matches(log.inner.as_ref()));
    }
}

fn assert_logs_ordered_and_not_removed(logs: &[alloy_rpc_types_eth::Log]) {
    let mut previous = None;
    for log in logs {
        assert!(!log.removed);
        let key = (
            log.block_number.expect("block number should be present"),
            log.transaction_index
                .expect("transaction index should be present"),
            log.log_index.expect("log index should be present"),
        );
        if let Some(previous) = previous {
            assert!(previous <= key);
        }
        previous = Some(key);
    }
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

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_with_cursor_and_filter() {
    let max_log_limit = 9;
    let nb_of_txs = 20;
    let nb_of_logs_per_tx: u32 = 7;

    let rollup_and_client =
        RollupAndClient::new(max_log_limit, EVM_EXTENSION.response_size_limit).await;
    let start_tx = rollup_and_client.get_tx_count().await as u32;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let mut nb_of_logs_received = 0;
    let mut logs_with_cursor = rollup_and_client
        .get_logs_with_cursor_and_filter(&serde_json::json!({
                "fromBlock": "0x0",
                "toBlock": "latest",
        }))
        .await;

    nb_of_logs_received += logs_with_cursor.logs.len();

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
    }

    assert_eq!(nb_of_logs_received as u32, nb_of_txs * nb_of_logs_per_tx);
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_with_cursor() {
    let max_log_limit = 9;
    let nb_of_txs = 20;
    let nb_of_logs_per_tx: u32 = 7;

    let rollup_and_client =
        RollupAndClient::new(max_log_limit, EVM_EXTENSION.response_size_limit).await;
    let start_tx = rollup_and_client.get_tx_count().await as u32;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let mut nb_of_logs_received = 0;
    let mut logs_with_cursor = rollup_and_client.get_logs_with_cursor(None).await;

    nb_of_logs_received += logs_with_cursor.logs.len();

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
    }

    assert_eq!(nb_of_logs_received as u32, nb_of_txs * nb_of_logs_per_tx);
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_at_max_response_size() {
    let nb_of_txs = 3;
    let nb_of_logs_per_tx: u32 = 40;

    // Set a low limit on response sizes to force pagination.
    let rollup_and_client = RollupAndClient::new(EVM_EXTENSION.max_log_limit, 1_000).await;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let mut nb_of_logs_received = 0;
    let mut logs_with_cursor = rollup_and_client.get_logs_with_cursor(None).await;

    nb_of_logs_received += logs_with_cursor.logs.len();

    let mut iters = 0;
    while let Some(packed_cursor) = logs_with_cursor.cursor {
        iters += 1;

        let cursor = Cursor::unpack(&packed_cursor).unwrap();
        logs_with_cursor = rollup_and_client.get_logs_with_cursor(Some(cursor)).await;
        nb_of_logs_received += logs_with_cursor.logs.len();
    }

    assert_eq!(nb_of_logs_received as u32, nb_of_txs * nb_of_logs_per_tx);
    assert!(iters > 1, "We should have reached the response size limit and been forced to paginate. This test might need adjusting, or pagination is broken.");
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

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let LogsWithMaybeCursor { mut logs, cursor } =
        rollup_and_client.get_logs_with_cursor(None).await;
    let cursor = cursor.map(|c| Cursor::unpack(&c).unwrap()).unwrap();
    assert_eq!(logs.len(), 1);
    let log = logs.pop().unwrap();
    assert_eq!(log.log.transaction_index, Some(0));
    assert_eq!(log.log.log_index, Some(0));
    assert_eq!(cursor.tx_index_absolute, 1);
    assert_eq!(cursor.log_index_in_tx, 1);

    let LogsWithMaybeCursor { mut logs, .. } =
        rollup_and_client.get_logs_with_cursor(Some(cursor)).await;
    assert_eq!(logs.len(), 1);
    let log = logs.pop().unwrap();
    assert_eq!(log.log.transaction_index, Some(0));
    assert_eq!(log.log.log_index, Some(1));
}

fn nb_of_logs_according_to_cursor(cursor: Cursor, nb_of_logs_per_tx: u32) -> u32 {
    (cursor.tx_index_absolute as u32) * nb_of_logs_per_tx + cursor.log_index_in_tx
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
    (test_rollup, evm_client, contract_a, contract_b)
}

struct RollupAndClient {
    test_rollup: TestRollup<MockDemoRollup<Native>>,
    client: SimpleStorageClient,
    contract_address: alloy_primitives::Address,
}

impl RollupAndClient {
    async fn new(max_log_limit: usize, response_size_limit: usize) -> RollupAndClient {
        let ext = SeqConfigExtension {
            max_log_limit,
            response_size_limit,
        };

        let (test_rollup, evm_client, _) = setup_with_simple_storage(0, ext).await;
        let contract_address = evm_client.alloy_deploy_contract().await;

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
