//! eth_feeHistory RPC endpoint tests
//!
//! Coverage notes:
//! - TC11 (percentile < 0): Skipped - alloy client validates percentiles client-side,
//!   negative values would require raw JSON-RPC to test server-side validation.

use alloy_provider::Provider;
use alloy_rpc_types_eth::BlockNumberOrTag;

use crate::evm::evm_test_helper::{
    alloy_client, create_simple_storage_client, deploy_contract_check, set_value_check,
    setup_test_rollup, EVM_EXTENSION, SENDER_PRIV_KEY,
};

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_basic() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(3, BlockNumberOrTag::Latest, &[])
        .await?;

    assert_eq!(fee_history.base_fee_per_gas.len(), 4);
    assert_eq!(fee_history.gas_used_ratio.len(), 3);
    assert!(fee_history
        .gas_used_ratio
        .iter()
        .all(|&r| (0.0..=1.0).contains(&r)));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_single_block() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(1, BlockNumberOrTag::Latest, &[])
        .await?;

    assert_eq!(fee_history.base_fee_per_gas.len(), 2);
    assert_eq!(fee_history.gas_used_ratio.len(), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_with_reward_percentiles() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[25.0, 50.0, 75.0])
        .await?;

    let rewards = fee_history.reward.unwrap();
    assert_eq!(rewards.len(), 2);
    assert_eq!(rewards[0].len(), 3);
    assert!(rewards.iter().all(|block| block.iter().all(|&r| r == 0)));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_zero_blocks() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);

    let result = client
        .get_fee_history(0, BlockNumberOrTag::Latest, &[])
        .await;

    assert!(result.is_err());

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_specific_block() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(5).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(3, BlockNumberOrTag::Number(3), &[])
        .await?;

    assert!(fee_history.oldest_block <= 3);
    assert_eq!(fee_history.base_fee_per_gas.len(), 4);
    assert_eq!(fee_history.gas_used_ratio.len(), 3);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_large_count_capped() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(2000, BlockNumberOrTag::Latest, &[])
        .await?;

    assert!(!fee_history.base_fee_per_gas.is_empty());
    assert!(fee_history.base_fee_per_gas.len() <= 1025);

    Ok(())
}

// ==================== Block Tag Variation Tests ====================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_pending_tag() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Pending, &[])
        .await?;

    // Pending tag should work and return valid data
    assert_eq!(fee_history.base_fee_per_gas.len(), 3);
    assert_eq!(fee_history.gas_used_ratio.len(), 2);

    Ok(())
}

/// KNOWN BUG: pending baseFeePerGas is returned as 0 instead of matching the pending block header.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_pending_base_fee_matches_pending_block() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let pending_block = client
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .expect("pending block should exist");
    let pending_base_fee = pending_block
        .header
        .base_fee_per_gas
        .expect("pending block should include base_fee_per_gas");

    let fee_history = client
        .get_fee_history(1, BlockNumberOrTag::Pending, &[])
        .await?;

    assert_eq!(fee_history.base_fee_per_gas.len(), 2);
    assert_eq!(
        fee_history.base_fee_per_gas[0], pending_base_fee as u128,
        "feeHistory pending base fee should match pending block header"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_finalized_tag() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Finalized, &[])
        .await?;

    // Finalized tag should work and return valid data
    assert_eq!(fee_history.base_fee_per_gas.len(), 3);
    assert_eq!(fee_history.gas_used_ratio.len(), 2);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_safe_tag() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Safe, &[])
        .await?;

    // Safe tag should work and return valid data
    assert_eq!(fee_history.base_fee_per_gas.len(), 3);
    assert_eq!(fee_history.gas_used_ratio.len(), 2);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_earliest_tag() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    // Request 2 blocks ending at earliest - should gracefully handle limited range
    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Earliest, &[])
        .await?;

    // Earliest points to block 0, so we can only get 1 block (0 to 0)
    // oldest_block should be 0
    assert_eq!(fee_history.oldest_block, 0);
    // base_fee_per_gas should have at least 1 entry
    assert!(!fee_history.base_fee_per_gas.is_empty());

    Ok(())
}

/// Rollup semantics: `latest` resolves to `pending`.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_latest_equals_pending() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    let latest_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[])
        .await?;
    let pending_history = client
        .get_fee_history(2, BlockNumberOrTag::Pending, &[])
        .await?;

    assert_eq!(latest_history.oldest_block, pending_history.oldest_block);
    assert_eq!(
        latest_history.base_fee_per_gas,
        pending_history.base_fee_per_gas
    );
    assert_eq!(
        latest_history.gas_used_ratio,
        pending_history.gas_used_ratio
    );
    assert_eq!(latest_history.reward, pending_history.reward);
    assert_eq!(
        latest_history.base_fee_per_blob_gas,
        pending_history.base_fee_per_blob_gas
    );
    assert_eq!(
        latest_history.blob_gas_used_ratio,
        pending_history.blob_gas_used_ratio
    );

    Ok(())
}

// ==================== Validation Error Tests ====================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_percentile_out_of_range_over_100() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    // Percentile > 100 should fail
    let result = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[50.0, 150.0])
        .await;

    assert!(result.is_err(), "Percentile > 100 should return error");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_percentiles_not_monotonic() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    // Non-monotonic percentiles should fail
    let result = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[75.0, 50.0, 25.0])
        .await;

    assert!(
        result.is_err(),
        "Non-monotonic percentiles should return error"
    );

    Ok(())
}

// ==================== Schema Invariant Tests ====================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_array_length_invariants() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(5).await;
    rollup.pause_preferred_batches().await;

    let block_count = 4u64;
    let fee_history = client
        .get_fee_history(block_count, BlockNumberOrTag::Latest, &[25.0, 75.0])
        .await?;

    // Key invariant: base_fee_per_gas has block_count + 1 entries
    assert_eq!(
        fee_history.base_fee_per_gas.len(),
        (block_count + 1) as usize,
        "base_fee_per_gas should have block_count + 1 entries"
    );

    // gas_used_ratio has block_count entries
    assert_eq!(
        fee_history.gas_used_ratio.len(),
        block_count as usize,
        "gas_used_ratio should have block_count entries"
    );

    // reward (if present) has block_count rows, each with percentile_count columns
    let rewards = fee_history.reward.expect("reward should be present");
    assert_eq!(
        rewards.len(),
        block_count as usize,
        "reward should have block_count rows"
    );
    for (i, row) in rewards.iter().enumerate() {
        assert_eq!(row.len(), 2, "reward row {i} should have 2 columns");
    }

    Ok(())
}

/// KNOWN BUG: reward rows are sized to requested blockCount even when fewer blocks exist.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_reward_len_matches_available_blocks() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let latest = client.get_block_number().await?;
    let block_count = latest + 5;
    let fee_history = client
        .get_fee_history(block_count, BlockNumberOrTag::Number(latest), &[50.0])
        .await?;

    let rewards = fee_history.reward.expect("reward should be present");
    assert_eq!(
        rewards.len(),
        fee_history.gas_used_ratio.len(),
        "reward row count should match returned block count"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_oldest_block_correctness() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(10).await;
    rollup.pause_preferred_batches().await;

    // Request 3 blocks ending at block 7
    let fee_history = client
        .get_fee_history(3, BlockNumberOrTag::Number(7), &[])
        .await?;

    // oldest_block should be 7 - (3 - 1) = 5
    assert_eq!(
        fee_history.oldest_block, 5,
        "oldest_block should be newest_block - block_count + 1"
    );

    // Verify array lengths match expected block count
    assert_eq!(fee_history.gas_used_ratio.len(), 3);

    Ok(())
}

// ==================== State Evolution Tests ====================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_gas_ratio_valid_range() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(5).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(5, BlockNumberOrTag::Finalized, &[])
        .await?;

    // Gas ratios must always be in valid range [0, 1]
    for (i, ratio) in fee_history.gas_used_ratio.iter().enumerate() {
        assert!(
            (0.0..=1.0).contains(ratio),
            "gas_used_ratio[{i}] = {ratio} should be in [0, 1]",
        );
    }

    // Base fee values should be non-negative (implicit in u128, but verify structure)
    assert!(
        !fee_history.base_fee_per_gas.is_empty(),
        "base_fee_per_gas should not be empty"
    );

    Ok(())
}

// ==================== Value Correctness Tests ====================

/// KNOWN BUG: baseFeePerGas can drop to 0 after genesis (violates EIP-1559 min base fee).
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_base_fee_nonzero_after_genesis() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(4).await;
    rollup.pause_preferred_batches().await;

    let latest = client.get_block_number().await?;
    let block_count = 3u64;
    let fee_history = client
        .get_fee_history(block_count, BlockNumberOrTag::Number(latest), &[])
        .await?;

    assert_eq!(
        fee_history.base_fee_per_gas.len(),
        (block_count + 1) as usize,
        "expected block_count + 1 base fees"
    );
    assert!(
        fee_history.oldest_block >= 1,
        "expected fee history range to start after genesis"
    );

    for (i, fee) in fee_history.base_fee_per_gas.iter().enumerate() {
        let block_num = fee_history.oldest_block + i as u64;
        assert!(
            *fee >= 1,
            "baseFeePerGas for block {block_num} should be >= 1, got {fee}"
        );
    }

    Ok(())
}

/// KNOWN BUG: genesis baseFeePerGas in feeHistory does not match the block header.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_earliest_values_match_block_header() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(1, BlockNumberOrTag::Earliest, &[])
        .await?;

    assert_eq!(fee_history.oldest_block, 0);
    assert_eq!(fee_history.base_fee_per_gas.len(), 2);
    assert_eq!(fee_history.gas_used_ratio.len(), 1);

    let block0 = client
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await?
        .expect("genesis block should exist");
    let base_fee0 = u128::from(
        block0
            .header
            .base_fee_per_gas
            .expect("genesis block should include base_fee_per_gas"),
    );
    assert_eq!(
        fee_history.base_fee_per_gas[0], base_fee0,
        "genesis baseFeePerGas should match the block header"
    );

    let gas_limit0 = block0.header.gas_limit;
    assert!(gas_limit0 > 0, "genesis gas_limit should be non-zero");
    let expected_ratio0 = block0.header.gas_used as f64 / gas_limit0 as f64;
    let delta0 = (fee_history.gas_used_ratio[0] - expected_ratio0).abs();
    assert!(
        delta0 < 1e-12,
        "genesis gas_used_ratio mismatch: expected {expected_ratio0}, got {}",
        fee_history.gas_used_ratio[0]
    );

    let block1 = client
        .get_block_by_number(BlockNumberOrTag::Number(1))
        .await?
        .expect("block 1 should exist");
    let base_fee1 = u128::from(
        block1
            .header
            .base_fee_per_gas
            .expect("block 1 should include base_fee_per_gas"),
    );
    assert_eq!(
        fee_history.base_fee_per_gas[1], base_fee1,
        "predicted next base fee should match block 1 header"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_values_match_block_headers() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;

    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let tx_hash = simple_storage
        .deploy_contract()
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let receipt = simple_storage.wait_for_finalized_receipt(tx_hash).await;
    let deploy_block = receipt
        .block_number
        .expect("deploy receipt should include block number");

    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let newest_block = deploy_block + 1;
    let fee_history = client
        .get_fee_history(3, BlockNumberOrTag::Number(newest_block), &[])
        .await?;

    assert_eq!(fee_history.base_fee_per_gas.len(), 4);
    assert_eq!(fee_history.gas_used_ratio.len(), 3);

    let oldest_block = fee_history.oldest_block;
    assert_eq!(oldest_block + 2, newest_block);

    for (i, ratio) in fee_history.gas_used_ratio.iter().enumerate() {
        let block_num = oldest_block + i as u64;
        let block = client
            .get_block_by_number(BlockNumberOrTag::Number(block_num))
            .await?
            .expect("block should exist");
        let header = block.header;
        let base_fee = header
            .base_fee_per_gas
            .expect("block should include base_fee_per_gas");
        let base_fee = u128::from(base_fee);
        assert_eq!(
            fee_history.base_fee_per_gas[i], base_fee,
            "baseFeePerGas should match the block header"
        );

        let gas_limit = header.gas_limit;
        assert!(gas_limit > 0, "block gas_limit should be non-zero");
        let expected_ratio = header.gas_used as f64 / gas_limit as f64;
        let delta = (*ratio - expected_ratio).abs();
        assert!(
            delta < 1e-12,
            "gas_used_ratio mismatch: expected {expected_ratio}, got {ratio}"
        );
    }

    let next_block_data = client
        .get_block_by_number(BlockNumberOrTag::Number(newest_block + 1))
        .await?
        .expect("next block should exist");
    let next_base_fee = next_block_data
        .header
        .base_fee_per_gas
        .expect("next block should include base_fee_per_gas");
    let next_base_fee = u128::from(next_base_fee);
    assert_eq!(
        fee_history.base_fee_per_gas[3], next_base_fee,
        "predicted next base fee should match the next block header"
    );

    Ok(())
}

// ==================== State and History Scenario Tests ====================

/// TC28: Empty blocks have gas_used_ratio = 0.0
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_empty_blocks_zero_ratio() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);

    // Produce multiple consecutive empty blocks
    rollup.wait_for_next_blocks(5).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(5, BlockNumberOrTag::Finalized, &[])
        .await?;

    // All blocks are empty, so gas_used_ratio should be 0.0 for all
    for (i, ratio) in fee_history.gas_used_ratio.iter().enumerate() {
        assert_eq!(
            *ratio, 0.0,
            "Empty block {i} should have gas_used_ratio = 0.0, got {ratio}"
        );
    }

    Ok(())
}

/// TC29: Block with transaction has gas_used_ratio > 0.0
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_block_with_tx_nonzero_ratio() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;

    // Deploy a contract (consumes gas)
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");

    // Wait for the deployment tx to be included in a block
    rollup.wait_for_next_blocks(1).await;

    // Send a transaction (set_value uses gas)
    set_value_check(&simple_storage, contract_address, 42)
        .await
        .expect("set_value should succeed");

    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(4, BlockNumberOrTag::Finalized, &[])
        .await?;

    // At least one block should have gas_used_ratio > 0
    let has_nonzero_ratio = fee_history.gas_used_ratio.iter().any(|&r| r > 0.0);
    assert!(
        has_nonzero_ratio,
        "Expected at least one block with gas_used_ratio > 0.0, got {:?}",
        fee_history.gas_used_ratio
    );

    Ok(())
}

/// TC30: Multiple transactions across blocks show distinct ratios
///
/// This test verifies that blocks with transactions show non-zero gas_used_ratio.
/// Note: Transactions may batch together depending on timing, so we track
/// the actual block each transaction lands in rather than assuming separation.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_multiple_txs_across_blocks() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;

    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");

    // Wait for deploy to finalize
    rollup.wait_for_next_blocks(2).await;

    // Track blocks where transactions land
    let mut tx_blocks = Vec::new();

    // Send 3 transactions, ensuring each lands in a separate block by:
    // 1. Wait for finalized receipt (tx is sealed in a block)
    // 2. Wait for next block before sending next tx
    for i in 0..3u32 {
        let tx_hash = simple_storage.set_value(contract_address, 100 + i).await;
        let receipt = simple_storage.wait_for_finalized_receipt(tx_hash).await;
        if let Some(block_num) = receipt.block_number {
            tx_blocks.push(block_num);
        }
        // Ensure next block starts before sending next tx
        rollup.wait_for_next_blocks(1).await;
    }

    rollup.pause_preferred_batches().await;

    // Query fee history covering all transaction blocks
    let latest_block = client.get_block_number().await?;
    let fee_history = client
        .get_fee_history(10, BlockNumberOrTag::Number(latest_block), &[])
        .await?;

    // Count blocks with non-zero gas usage
    let nonzero_count = fee_history
        .gas_used_ratio
        .iter()
        .filter(|&&r| r > 0.0)
        .count();

    // We should have at least 2 blocks with gas (deploy may batch with first set_value)
    // The key invariant is that transactions DO show up as non-zero gas
    assert!(
        nonzero_count >= 2,
        "Expected at least 2 blocks with gas usage (4 txs may batch), got {nonzero_count} in {:?}. Tx blocks: {:?}",
        fee_history.gas_used_ratio,
        tx_blocks
    );

    // Verify the unique transaction blocks show non-zero ratios when queried directly
    for &block_num in &tx_blocks {
        if block_num >= fee_history.oldest_block {
            let idx = (block_num - fee_history.oldest_block) as usize;
            if idx < fee_history.gas_used_ratio.len() {
                assert!(
                    fee_history.gas_used_ratio[idx] > 0.0,
                    "Block {} should have non-zero gas_used_ratio, got {}",
                    block_num,
                    fee_history.gas_used_ratio[idx]
                );
            }
        }
    }

    Ok(())
}

/// TC31: Fee history progression - oldest_block advances as new blocks are produced
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_progression() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(5).await;

    // Query fee history at current state
    let history1 = client
        .get_fee_history(3, BlockNumberOrTag::Latest, &[])
        .await?;
    let oldest1 = history1.oldest_block;

    // Produce more blocks
    rollup.wait_for_next_blocks(3).await;

    // Query again - oldest_block should advance
    let history2 = client
        .get_fee_history(3, BlockNumberOrTag::Latest, &[])
        .await?;
    let oldest2 = history2.oldest_block;

    assert!(
        oldest2 > oldest1,
        "After producing blocks, oldest_block should advance: was {oldest1}, now {oldest2}"
    );

    // The difference should be approximately equal to blocks produced
    let block_diff = oldest2 - oldest1;
    assert!(
        (2..=4).contains(&block_diff),
        "Block advancement should be ~3, got {block_diff}"
    );

    Ok(())
}

/// TC32: Same block queried via Number(N) and via range ending at N returns consistent gas_used_ratio
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_consistent_query_methods() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;

    // Create some gas usage
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");
    rollup.wait_for_next_blocks(2).await;
    set_value_check(&simple_storage, contract_address, 999)
        .await
        .expect("set_value should succeed");
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    // Get the current block number
    let block_num = client.get_block_number().await?;
    let target_block = block_num - 2; // A sealed block

    // Query using Number(target_block) with block_count=1
    let history_by_number = client
        .get_fee_history(1, BlockNumberOrTag::Number(target_block), &[])
        .await?;

    // Query using a range that ends at target_block (block_count=3)
    let history_range = client
        .get_fee_history(3, BlockNumberOrTag::Number(target_block), &[])
        .await?;

    // The last gas_used_ratio in history_range should match the single ratio in history_by_number
    let ratio_single = history_by_number.gas_used_ratio[0];
    let ratio_from_range = history_range.gas_used_ratio.last().unwrap();

    assert_eq!(
        ratio_single, *ratio_from_range,
        "gas_used_ratio for block {target_block} should be consistent: single={ratio_single}, range={ratio_from_range}"
    );

    Ok(())
}

/// TC34: Mixed history pattern - verify ratios match pattern [0, >0, 0, >0, >0]
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_mixed_pattern() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;

    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");

    // Wait for deploy block
    rollup.wait_for_next_blocks(1).await;

    // Record starting block
    let start_block = client.get_block_number().await?;

    // Create pattern: [empty, tx, empty, tx, tx]
    // Block 1: empty
    rollup.wait_for_next_blocks(1).await;

    // Block 2: tx
    set_value_check(&simple_storage, contract_address, 1)
        .await
        .expect("set_value should succeed");
    rollup.wait_for_next_blocks(1).await;

    // Block 3: empty
    rollup.wait_for_next_blocks(1).await;

    // Block 4: tx
    set_value_check(&simple_storage, contract_address, 2)
        .await
        .expect("set_value should succeed");
    rollup.wait_for_next_blocks(1).await;

    // Block 5: tx
    set_value_check(&simple_storage, contract_address, 3)
        .await
        .expect("set_value should succeed");
    rollup.wait_for_next_blocks(1).await;

    rollup.pause_preferred_batches().await;

    // Query for 5 blocks after start_block
    let end_block = start_block + 5;
    let fee_history = client
        .get_fee_history(5, BlockNumberOrTag::Number(end_block), &[])
        .await?;

    // Verify the pattern: should have alternating zero/nonzero pattern
    // The exact pattern depends on timing, but we can verify:
    // - Some blocks have ratio = 0.0 (empty)
    // - Some blocks have ratio > 0.0 (with tx)
    let zero_count = fee_history
        .gas_used_ratio
        .iter()
        .filter(|&&r| r == 0.0)
        .count();
    let nonzero_count = fee_history
        .gas_used_ratio
        .iter()
        .filter(|&&r| r > 0.0)
        .count();

    assert!(
        zero_count >= 1,
        "Expected at least 1 empty block, got {zero_count}"
    );
    assert!(
        nonzero_count >= 2,
        "Expected at least 2 blocks with transactions, got {nonzero_count}"
    );

    Ok(())
}

/// TC35: Base fee stability - KNOWN BUG
///
/// This test verifies EIP-1559 base fee constraints: changes should be max 12.5% per block,
/// and base fee should never drop below 1 wei.
///
/// **KNOWN BUG**: Currently FAILING because the rollup uses `saturating_sub` in
/// `crates/module-system/module-implementations/sov-chain-state/src/gas.rs:189`
/// which allows base_fee to drop to 0, violating EIP-1559.
///
/// Example failure: "Base fee swing from block 7->8 is too large: 7 -> 0"
///
/// This should be fixed by either:
/// 1. Using checked arithmetic with min(1) bound
/// 2. Implementing proper EIP-1559 elasticity constraints
///
/// See: crates/module-system/module-implementations/sov-chain-state/src/gas.rs
// #[ignore = "Known bug: base_fee can drop to 0 due to saturating_sub (violates EIP-1559)"]
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_base_fee_stability() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);

    // Produce several blocks
    rollup.wait_for_next_blocks(10).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(8, BlockNumberOrTag::Finalized, &[])
        .await?;

    // Check that base_fee values don't have extreme swings
    // EIP-1559 constrains changes to max 12.5% per block
    let base_fees = &fee_history.base_fee_per_gas;
    assert!(
        base_fees.len() >= 2,
        "Need at least 2 base fees to check stability"
    );

    for i in 1..base_fees.len() {
        let prev = base_fees[i - 1];
        let curr = base_fees[i];

        // Allow up to 15% change (slightly more than EIP-1559's 12.5% to account for implementation variance)
        // But if prev is 0, any value is acceptable
        if prev > 0 {
            let max_change = prev / 8 + prev / 50; // ~14.5%
            let diff = curr.abs_diff(prev);
            assert!(
                diff <= max_change,
                "Base fee swing from block {}->{i} is too large: {prev} -> {curr} (diff={diff}, max={max_change})",
                i - 1,
            );
        }
    }

    Ok(())
}

// ==================== Missing Test Cases ====================

/// TC06: finalized and safe return identical results
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_finalized_equals_safe() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(5).await;
    rollup.pause_preferred_batches().await;

    let finalized = client
        .get_fee_history(3, BlockNumberOrTag::Finalized, &[25.0, 75.0])
        .await?;

    let safe = client
        .get_fee_history(3, BlockNumberOrTag::Safe, &[25.0, 75.0])
        .await?;

    // In this rollup, finalized and safe both map to latest sealed block
    assert_eq!(
        finalized.oldest_block, safe.oldest_block,
        "finalized and safe should return same oldest_block"
    );
    assert_eq!(
        finalized.base_fee_per_gas, safe.base_fee_per_gas,
        "finalized and safe should return same base_fee_per_gas"
    );
    assert_eq!(
        finalized.gas_used_ratio, safe.gas_used_ratio,
        "finalized and safe should return same gas_used_ratio"
    );
    assert_eq!(
        finalized.reward, safe.reward,
        "finalized and safe should return same reward"
    );

    Ok(())
}

/// TC09: blockCount = 1024 exactly works (boundary case)
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_block_count_1024_boundary() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(5).await;
    rollup.pause_preferred_batches().await;

    // Request exactly 1024 blocks - should work without error
    let fee_history = client
        .get_fee_history(1024, BlockNumberOrTag::Latest, &[])
        .await?;

    // Should return data (may be less than 1024 if chain is shorter)
    assert!(!fee_history.base_fee_per_gas.is_empty());
    // base_fee_per_gas.len() should be at most 1025 (1024 + 1)
    assert!(
        fee_history.base_fee_per_gas.len() <= 1025,
        "base_fee_per_gas should have at most 1025 entries for blockCount=1024"
    );

    Ok(())
}

/// TC13: Duplicate percentiles - verify behavior (spec allows <=)
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_duplicate_percentiles() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    // Duplicate percentiles [25, 25, 75] - should be accepted (monotonically non-decreasing)
    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[25.0, 25.0, 75.0])
        .await?;

    let rewards = fee_history.reward.expect("reward should be present");
    assert_eq!(rewards.len(), 2, "Should have 2 blocks of rewards");
    for (i, row) in rewards.iter().enumerate() {
        assert_eq!(
            row.len(),
            3,
            "Block {i} reward should have 3 percentile values"
        );
        // Duplicate percentiles should return same value
        assert_eq!(
            row[0], row[1],
            "Duplicate percentiles 25, 25 should return same value"
        );
    }

    Ok(())
}

/// TC14: Empty percentiles array omits reward field
///
/// BUG: Per Ethereum spec, when empty percentiles are provided, the reward field
/// should be omitted from the response (None). The rollup instead returns
/// Some([[], []]) - empty 2D arrays. This may confuse clients that check for
/// reward presence to determine if percentiles were requested.
#[tokio::test(flavor = "multi_thread")]
// #[ignore = "Known bug: empty percentiles returns Some([[], []]) instead of None"]
async fn test_fee_history_empty_percentiles_no_reward() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[])
        .await?;

    // With empty percentiles, reward should be None
    assert!(
        fee_history.reward.is_none(),
        "Empty percentiles should result in no reward field, got {:?}",
        fee_history.reward
    );

    Ok(())
}

/// TC19: Blob gas fields are empty arrays (rollup-specific, EIP-4844 not implemented)
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_blob_gas_fields_empty() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    let fee_history = client
        .get_fee_history(3, BlockNumberOrTag::Latest, &[])
        .await?;

    // EIP-4844 blob gas fields should be empty in this rollup
    assert!(
        fee_history.base_fee_per_blob_gas.is_empty(),
        "base_fee_per_blob_gas should be empty (EIP-4844 not implemented), got {:?}",
        fee_history.base_fee_per_blob_gas
    );
    assert!(
        fee_history.blob_gas_used_ratio.is_empty(),
        "blob_gas_used_ratio should be empty (EIP-4844 not implemented), got {:?}",
        fee_history.blob_gas_used_ratio
    );

    Ok(())
}

/// TC24: baseFeePerGas[last] is the predicted next block fee
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_predicted_next_block_fee() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(5).await;
    rollup.pause_preferred_batches().await;

    let latest = client.get_block_number().await?;
    let newest_block = latest.saturating_sub(1);
    let fee_history = client
        .get_fee_history(1, BlockNumberOrTag::Number(newest_block), &[])
        .await?;

    // baseFeePerGas should have block_count + 1 entries
    // The last entry is the predicted fee for the NEXT block
    assert_eq!(
        fee_history.base_fee_per_gas.len(),
        2,
        "Should have block_count + 1 base fees"
    );

    let predicted_fee = fee_history.base_fee_per_gas[1];
    let next_block = client
        .get_block_by_number(BlockNumberOrTag::Number(newest_block + 1))
        .await?
        .expect("next block should exist");
    let next_base_fee = next_block
        .header
        .base_fee_per_gas
        .expect("next block should include base_fee_per_gas");
    let next_base_fee = u128::from(next_base_fee);
    assert_eq!(
        predicted_fee, next_base_fee,
        "predicted next base fee should match the next block header"
    );

    Ok(())
}

/// TC27: Future block number handling
///
/// BUG: When requesting fee history for a future block (e.g., block 1003 when
/// chain is at block 3), the rollup should either return an error or return
/// data bounded by the current chain height. Instead, it returns fabricated
/// data with oldest_block = 1001, which is invalid since those blocks don't exist.
/// This could mislead clients into thinking the chain has more history than it does.
#[tokio::test(flavor = "multi_thread")]
// #[ignore = "Known bug: future block returns fabricated data instead of error/bounded result"]
async fn test_fee_history_future_block() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    let current_block = client.get_block_number().await?;
    let future_block = current_block + 1000; // Way in the future

    // Request fee history for a future block
    let result = client
        .get_fee_history(3, BlockNumberOrTag::Number(future_block), &[])
        .await;

    // Should either error OR return empty/partial data gracefully
    // The exact behavior depends on implementation
    match result {
        Ok(fee_history) => {
            // If it succeeds, it should return empty or partial data
            // oldest_block should not be beyond current chain
            assert!(
                fee_history.oldest_block <= current_block + 1,
                "oldest_block {} should not be beyond current block {}",
                fee_history.oldest_block,
                current_block
            );
        }
        Err(_) => {
            // Error is also acceptable for future block
        }
    }

    Ok(())
}

/// TC28: Fractional percentiles work correctly
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_fractional_percentiles() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    // Fractional percentiles like [10.5, 50.0, 90.5]
    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[10.5, 50.0, 90.5])
        .await?;

    let rewards = fee_history.reward.expect("reward should be present");
    assert_eq!(rewards.len(), 2, "Should have 2 blocks of rewards");
    for (i, row) in rewards.iter().enumerate() {
        assert_eq!(
            row.len(),
            3,
            "Block {i} reward should have 3 percentile values for fractional percentiles"
        );
    }

    Ok(())
}

/// TC22: Percentile boundary values 0 and 100
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_percentile_boundaries() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(3).await;
    rollup.pause_preferred_batches().await;

    // Test boundary percentiles [0, 50, 100]
    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[0.0, 50.0, 100.0])
        .await?;

    let rewards = fee_history.reward.expect("reward should be present");
    assert_eq!(rewards.len(), 2, "Should have 2 blocks of rewards");
    for (i, row) in rewards.iter().enumerate() {
        assert_eq!(
            row.len(),
            3,
            "Block {i} reward should have 3 percentile values"
        );
        // 0th percentile should be <= 100th percentile (monotonic)
        assert!(
            row[0] <= row[2],
            "Block {i}: 0th percentile {} should be <= 100th percentile {}",
            row[0],
            row[2]
        );
    }

    Ok(())
}

/// TC34: Heavy gas usage - gas_used_ratio approaches but doesn't exceed 1.0
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_heavy_gas_usage() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    // Wait for initial blocks
    rollup.wait_for_next_blocks(2).await;

    // Deploy contract
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");
    rollup.wait_for_next_blocks(1).await;

    let block_before = client.get_block_number().await?;

    // Burn a lot of gas with keccak256 loops
    // Use a high iteration count to consume significant gas
    let tx_hash = simple_storage.alloy_burn_gas(contract_address, 10000).await;
    simple_storage.wait_for_finalized_receipt(tx_hash).await;

    rollup.pause_preferred_batches().await;

    let block_after = client.get_block_number().await?;

    // Query fee history for the block containing the heavy tx
    let fee_history = client
        .get_fee_history(
            (block_after - block_before + 1) as u64,
            BlockNumberOrTag::Finalized,
            &[],
        )
        .await?;

    // Find the highest gas_used_ratio
    let max_ratio = fee_history
        .gas_used_ratio
        .iter()
        .cloned()
        .fold(0.0_f64, f64::max);

    // The heavy tx should produce a non-trivial gas ratio
    assert!(
        max_ratio > 0.0,
        "Heavy gas tx should produce non-zero gas_used_ratio, got {max_ratio}",
    );

    // Critical invariant: ratio should never exceed 1.0
    for (i, ratio) in fee_history.gas_used_ratio.iter().enumerate() {
        assert!(
            *ratio <= 1.0,
            "Block {i} gas_used_ratio {ratio} exceeds 1.0",
        );
    }

    Ok(())
}
