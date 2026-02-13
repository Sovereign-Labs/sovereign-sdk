use alloy_primitives::utils::parse_ether;
use alloy_primitives::{Address, BlockHash, U256, U64};
use alloy_provider::DynProvider;
use alloy_provider::Provider;
use alloy_rpc_types_eth::BlockId;
use alloy_rpc_types_eth::BlockNumberOrTag;
use alloy_rpc_types_eth::BlockNumberOrTag::{Earliest, Latest, Pending};
use alloy_rpc_types_eth::BlockTransactions;
use alloy_rpc_types_eth::Header;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use sov_evm_test_utils::Erc20;
use sov_evm_test_utils::Submit;
use std::time::Duration;

use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;

async fn by_number(
    client: &DynProvider,
    tag: impl Into<BlockNumberOrTag>,
) -> anyhow::Result<Option<Header>> {
    Ok(client
        .get_block_by_number(tag.into())
        .await?
        .map(|block| block.header))
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_by_number() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.pause_preferred_batches().await;

    assert_eq!(by_number(&client, Earliest).await?.unwrap().number, 0);
    assert_eq!(by_number(&client, Latest).await?.unwrap().number, 0);
    assert_eq!(by_number(&client, Pending).await?.unwrap().number, 0);
    assert_eq!(by_number(&client, 1).await?, None);
    assert_eq!(by_number(&client, 2).await?, None);

    rollup.resume_preferred_batches().await;
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    assert_eq!(by_number(&client, Earliest).await?.unwrap().number, 0);
    assert_eq!(by_number(&client, Latest).await?.unwrap().number, 1);
    assert_eq!(by_number(&client, Pending).await?.unwrap().number, 1);

    assert_eq!(by_number(&client, 1).await?.unwrap().number, 1);
    assert_eq!(by_number(&client, 2).await?, None);
    assert_eq!(by_number(&client, 3).await?, None);

    assert_eq!(
        by_number(&client, 1).await?.unwrap().parent_hash,
        by_number(&client, 0).await?.unwrap().hash
    );

    Ok(())
}

async fn by_hash(client: &DynProvider, hash: BlockHash) -> anyhow::Result<Option<Header>> {
    Ok(client
        .get_block_by_hash(hash)
        .await?
        .map(|block| block.header))
}

async fn wait_for_latest_with_min_txs(
    client: &DynProvider,
    min_txs: usize,
) -> anyhow::Result<(BlockHash, u64)> {
    for _ in 0..50 {
        let latest = client
            .get_block_by_number(Latest)
            .await?
            .ok_or_else(|| anyhow::anyhow!("latest block should exist"))?;
        let tx_count = match latest.transactions {
            BlockTransactions::Hashes(hashes) => hashes.len(),
            BlockTransactions::Full(txs) => txs.len(),
            BlockTransactions::Uncle => 0,
        };
        if tx_count >= min_txs {
            return Ok((latest.header.hash, tx_count as u64));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    anyhow::bail!("latest block did not include the expected pending transactions")
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_by_hash() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);

    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let latest_hash = by_number(&client, Latest).await?.unwrap().parent_hash;
    let latest = by_hash(&client, latest_hash).await?.unwrap();
    assert_eq!(latest.hash, latest_hash);
    assert_eq!(latest.number, 1);

    let pending_hash = by_number(&client, Latest).await?.unwrap().hash;
    assert_ne!(pending_hash, BlockHash::ZERO);
    // Because the hash of the pending block is fake - it can't be fetched by hash
    assert_ne!(by_hash(&client, pending_hash).await?, None);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_transaction_count_by_hash_accepts_synthetic_hash() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;
    usdc.mint(Address::ZERO, parse_ether("1")?).submit().await?;
    usdc.mint(Address::ZERO, parse_ether("1")?).submit().await?;

    let (latest_hash, latest_tx_count) = wait_for_latest_with_min_txs(&client, 3).await?;
    let by_hash_count = client
        .get_block_transaction_count_by_hash(latest_hash)
        .await?;

    assert_eq!(
        by_hash_count,
        Some(latest_tx_count),
        "latest should resolve to the pending synthetic block while txs are pending"
    );

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
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let header = by_number(&client, 0).await?.unwrap();
    // Block size is 508 bytes with gas_limit = 100_000_000_000 (5-byte RLP encoding)
    // Previously was 507 bytes with gas_limit = 1_000_000_000 (4-byte RLP encoding)
    assert_eq!(header.size.unwrap().to::<u64>(), 508);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_storage_at_returns_32_byte_data_hex() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let client = crate::evm::evm_test_helper::create_simple_storage_client(
        rollup.http_addr,
        crate::evm::evm_test_helper::SENDER_PRIV_KEY,
    )
    .await;

    let contract_address = crate::evm::evm_test_helper::deploy_contract_check(&client)
        .await
        .expect("contract deployment should succeed");
    crate::evm::evm_test_helper::set_value_check(&client, contract_address, 1)
        .await
        .expect("setting storage should succeed");

    // Query raw JSON-RPC output to assert Ethereum DATA shape (fixed 32-byte hex).
    let raw: String = client
        .rpc_client
        .ws
        .request(
            "eth_getStorageAt",
            rpc_params![contract_address, U256::from(0)],
        )
        .await?;

    assert_eq!(
        raw.len(),
        66,
        "eth_getStorageAt should return 0x-prefixed 32-byte hex data"
    );
    assert!(
        raw.starts_with("0x"),
        "eth_getStorageAt should return a hex string"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_transaction_count_by_hash_accepts_synthetic_hash() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let ws_client = crate::evm::evm_test_helper::create_simple_storage_client(
        rollup.http_addr,
        crate::evm::evm_test_helper::SENDER_PRIV_KEY,
    )
    .await;
    ws_client.send_eth(Address::ZERO, U256::from(1)).await;

    let client = alloy_client(rollup.http_addr);
    let latest = by_number(&client, Latest)
        .await?
        .expect("latest block should exist");
    let sealed_height = client.get_block_number().await?;
    assert_eq!(
        latest.number,
        sealed_height + 1,
        "latest should resolve to the pending synthetic block while tx is pending"
    );

    let by_hash: Option<U64> = ws_client
        .rpc_client
        .ws
        .request(
            "eth_getBlockTransactionCountByHash",
            rpc_params![latest.hash],
        )
        .await?;
    let by_number: Option<U64> = ws_client
        .rpc_client
        .ws
        .request(
            "eth_getBlockTransactionCountByNumber",
            rpc_params!["latest"],
        )
        .await?;

    assert_eq!(
        by_hash, by_number,
        "hash and number forms should agree for the same synthetic latest block"
    );
    assert!(
        by_hash.is_some(),
        "synthetic hash should be queryable by eth_getBlockTransactionCountByHash"
    );

    Ok(())
}
