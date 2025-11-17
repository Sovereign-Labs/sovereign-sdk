use alloy_primitives::utils::parse_ether;
use alloy_primitives::{Address, BlockHash};
use alloy_provider::DynProvider;
use alloy_provider::Provider;
use alloy_rpc_types_eth::BlockId;
use alloy_rpc_types_eth::BlockNumberOrTag;
use alloy_rpc_types_eth::BlockNumberOrTag::{Earliest, Latest, Pending};
use alloy_rpc_types_eth::Header;
use sov_test_utils::Erc20;
use sov_test_utils::Submit;

use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;

async fn by_number(client: &DynProvider, tag: BlockNumberOrTag) -> anyhow::Result<Option<Header>> {
    Ok(client
        .get_block_by_number(tag)
        .await?
        .map(|block| block.header))
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_by_number() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.pause_preferred_batches().await;

    assert_eq!(by_number(&client, Earliest).await?.unwrap().number, 0);
    assert_eq!(by_number(&client, Latest).await?.unwrap().number, 1);
    assert_eq!(by_number(&client, Pending).await?.unwrap().number, 1);
    assert_eq!(by_number(&client, 1.into()).await?.unwrap().number, 1);
    assert_eq!(by_number(&client, 2.into()).await?, None);

    rollup.resume_preferred_batches().await;
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    assert_eq!(by_number(&client, Earliest).await?.unwrap().number, 0);
    assert_eq!(by_number(&client, Latest).await?.unwrap().number, 2);
    assert_eq!(by_number(&client, Pending).await?.unwrap().number, 2);

    assert_eq!(by_number(&client, 1.into()).await?.unwrap().number, 1);
    assert_eq!(by_number(&client, 2.into()).await?.unwrap().number, 2);
    assert_eq!(by_number(&client, 3.into()).await?, None);

    assert_eq!(
        by_number(&client, 2.into()).await?.unwrap().parent_hash,
        by_number(&client, 1.into()).await?.unwrap().hash
    );

    Ok(())
}

async fn by_hash(client: &DynProvider, hash: BlockHash) -> anyhow::Result<Option<Header>> {
    Ok(client
        .get_block_by_hash(hash)
        .await?
        .map(|block| block.header))
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_by_hash() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);

    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let latest_hash = by_number(&client, Pending).await?.unwrap().parent_hash;
    let latest = by_hash(&client, latest_hash).await?.unwrap();
    assert_eq!(latest.hash, latest_hash);
    assert_eq!(latest.number, 1);

    let pending_hash = by_number(&client, Pending).await?.unwrap().hash;
    assert_eq!(pending_hash, BlockHash::ZERO);
    // Because the hash of the pending block is fake - it can't be fetched by hash
    assert_eq!(by_hash(&client, pending_hash).await?, None);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_receipts() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;

    let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;
    usdc.mint(Address::ZERO, parse_ether("1")?).submit().await?;
    rollup.pause_preferred_batches().await;

    let empty_block_receipts = client.get_block_receipts(BlockId::from(1)).await?.unwrap();
    assert_eq!(empty_block_receipts.len(), 0);

    let mut pending_receipts = client
        .get_block_receipts(BlockId::pending())
        .await?
        .unwrap();
    assert_eq!(pending_receipts.len(), 2);

    let deployment_receipt = pending_receipts.remove(0);
    assert!(deployment_receipt.contract_address.is_some());
    assert_eq!(deployment_receipt.transaction_index, Some(0));
    assert_eq!(deployment_receipt.logs().len(), 0);

    let mint_receipt = pending_receipts.remove(0);
    assert!(mint_receipt.contract_address.is_none());
    assert_eq!(mint_receipt.transaction_index, Some(1));
    assert_eq!(mint_receipt.logs().len(), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn block_size() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.pause_preferred_batches().await;

    let header = by_number(&client, Latest).await?.unwrap();
    assert_eq!(header.size.unwrap().to::<u64>(), 503);

    Ok(())
}
