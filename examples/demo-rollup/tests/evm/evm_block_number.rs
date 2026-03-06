use crate::evm::evm_test_helper::{
    alloy_client, deploy_contract_check, setup_test_rollup, setup_with_simple_storage,
    EVM_EXTENSION,
};
use alloy_provider::Provider;
use alloy_rpc_types_eth::BlockNumberOrTag;

#[tokio::test(flavor = "multi_thread")]
async fn pending_block_number() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(1).await;
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

#[tokio::test(flavor = "multi_thread")]
async fn block_number_matches_latest_when_pending_exists() -> anyhow::Result<()> {
    let (rollup, simple_storage, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    rollup.pause_preferred_batches().await;

    // eth_blockNumber may already point to pending, so use finalized as the sealed reference.
    let sealed_head_number = client
        .get_block_by_number(BlockNumberOrTag::Finalized)
        .await?
        .expect("finalized block should exist")
        .header
        .number;
    let tx_hash = simple_storage.set_value(contract_address, 1).await;
    simple_storage.wait_for_receipt(tx_hash).await;

    let latest = client
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .expect("latest block should exist");
    let pending = client
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .expect("pending block should exist");
    let eth_block_number = client.get_block_number().await?;

    assert_eq!(latest.header.number, pending.header.number);
    assert_eq!(
        eth_block_number, latest.header.number,
        "eth_blockNumber should align with eth_getBlockByNumber(latest)"
    );
    assert_eq!(
        eth_block_number,
        sealed_head_number + 1,
        "eth_blockNumber should point to the pending head (sealed + 1) when pending tx exists"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn block_number_matches_pending_receipt_and_transaction() -> anyhow::Result<()> {
    let (rollup, simple_storage, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    rollup.pause_preferred_batches().await;

    let tx_hash = simple_storage.set_value(contract_address, 2).await;
    let receipt = simple_storage.wait_for_receipt(tx_hash).await;
    let tx = simple_storage
        .transaction(tx_hash)
        .await
        .expect("transaction should be available while pending");
    let eth_block_number = client.get_block_number().await?;

    let receipt_block_number = receipt
        .block_number
        .expect("receipt should include blockNumber while pending");
    let tx_block_number = tx
        .block_number
        .expect("transaction should include blockNumber while pending");

    assert_eq!(receipt_block_number, tx_block_number);
    assert_eq!(
        eth_block_number, receipt_block_number,
        "eth_blockNumber should align with pending receipt.blockNumber"
    );
    assert_eq!(
        eth_block_number, tx_block_number,
        "eth_blockNumber should align with pending transaction.blockNumber"
    );

    Ok(())
}
