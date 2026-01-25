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

    assert!(client
        .get_fee_history(2000, BlockNumberOrTag::Latest, &[])
        .await
        .is_err());

    Ok(())
}
