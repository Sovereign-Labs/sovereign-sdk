use crate::evm::evm_test_helper::{alloy_client, setup_test_rollup, EVM_EXTENSION};
use alloy_provider::Provider;
use alloy_rpc_types_eth::BlockNumberOrTag;

#[tokio::test(flavor = "multi_thread")]
async fn pending_block_number() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let pending_block = client
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .unwrap();

    let pending_header = pending_block.header;
    assert!(pending_header.gas_limit > 0);
    assert!(pending_header.number > 0);
    assert!(pending_header.timestamp > 0);
    assert!(pending_header.base_fee_per_gas.unwrap() > 0);
    assert_eq!(pending_header.gas_used, 0);

    Ok(())
}
