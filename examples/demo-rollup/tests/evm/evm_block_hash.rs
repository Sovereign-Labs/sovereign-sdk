use alloy_primitives::{B256, U256};
use alloy_provider::Provider;
use sov_test_utils::BlockHash;

use crate::evm::evm_test_helper::{alloy_client, setup_test_rollup, EVM_EXTENSION};

#[tokio::test(flavor = "multi_thread")]
async fn block_hash() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;
    let number = client.get_block_number().await?;
    let block_hash = BlockHash::deploy(client.clone()).await?;

    let latest = block_hash.block_hash(U256::from(number)).call().await?;
    let parent = block_hash.block_hash(U256::from(number - 1)).call().await?;
    let grand_parent = block_hash.block_hash(U256::from(number - 2)).call().await?;

    assert_eq!(latest, B256::ZERO);
    assert!(parent != B256::ZERO);
    assert!(grand_parent != B256::ZERO);
    assert!(parent != grand_parent);

    Ok(())
}
