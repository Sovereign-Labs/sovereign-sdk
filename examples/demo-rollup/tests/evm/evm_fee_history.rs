use alloy_provider::Provider;
use alloy_rpc_types_eth::BlockNumberOrTag;

use crate::evm::evm_test_helper::{alloy_client, setup_test_rollup, EVM_EXTENSION};

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

    // In this implementation, latest and pending resolve to the same block
    // Document this as a known semantic difference from Ethereum spec
    assert_eq!(latest_history.oldest_block, pending_history.oldest_block);
    assert_eq!(
        latest_history.base_fee_per_gas.len(),
        pending_history.base_fee_per_gas.len()
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
