use alloy_primitives::{B256, U256};
use alloy_provider::Provider;
use alloy_rpc_types_eth::BlockId;
use sov_evm_test_utils::BlockHash;

use crate::evm::evm_test_helper::{alloy_client, setup_test_rollup, EVM_EXTENSION};

#[tokio::test(flavor = "multi_thread")]
async fn block_hash() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;
    let block_hash = BlockHash::deploy(client.clone()).await?;
    rollup.pause_preferred_batches().await;
    let pending_number = client
        .get_block(BlockId::pending())
        .await?
        .unwrap()
        .number();

    let pending_block_hash = block_hash
        .block_hash(U256::from(pending_number))
        .block(BlockId::pending())
        .call()
        .await?;
    let parent_block_hash = block_hash
        .block_hash(U256::from(pending_number - 1))
        .block(BlockId::pending())
        .call()
        .await?;
    let grand_parent_block_hash = block_hash
        .block_hash(U256::from(pending_number - 2))
        .block(BlockId::pending())
        .call()
        .await?;

    assert_eq!(pending_block_hash, B256::ZERO);
    assert!(parent_block_hash != B256::ZERO);
    assert!(grand_parent_block_hash != B256::ZERO);
    assert!(parent_block_hash != grand_parent_block_hash);
    assert_eq!(
        parent_block_hash,
        client
            .get_block_by_number((pending_number - 1).into())
            .await?
            .unwrap()
            .hash()
    );
    assert_eq!(
        grand_parent_block_hash,
        client
            .get_block_by_number((pending_number - 2).into())
            .await?
            .unwrap()
            .hash()
    );

    Ok(())
}
