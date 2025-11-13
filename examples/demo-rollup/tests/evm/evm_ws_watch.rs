use std::time::Duration;

use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use alloy_primitives::{Address, B256};
use alloy_rpc_types_eth::BlockNumberOrTag;
use futures::StreamExt;
use tokio::time::timeout;

use crate::evm::evm_test_helper::alloy_ws_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;

#[tokio::test(flavor = "multi_thread")]
async fn ws_watch_returns_receipt() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_ws_client(rollup.http_addr).await;

    let tx = TransactionRequest::default().with_to(Address::ZERO);

    let pending = client.send_transaction(tx).await?;

    // This should complete very quickly (not hang)
    // If this times out, it proves WebSocket .watch() is broken
    let tx_hash = timeout(Duration::from_secs(1), pending.watch()).await??;

    assert_ne!(tx_hash, B256::ZERO);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ws_get_receipt() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_ws_client(rollup.http_addr).await;

    let tx = TransactionRequest::default().with_to(Address::ZERO);
    let pending = client.send_transaction(tx).await?;
    let hash = *pending.tx_hash();

    let confirmed_hash = pending.watch().await?;

    assert_eq!(confirmed_hash, hash);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ws_subscribe_new_heads() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_ws_client(rollup.http_addr).await;
    let subscription = client.subscribe_blocks().await?;
    rollup.wait_for_next_blocks(3).await;

    let headers: Vec<_> = subscription.into_stream().take(3).collect().await;

    assert_eq!(headers.len(), 3);
    assert_eq!(headers[1].number, headers[0].number + 1);
    assert_eq!(headers[2].number, headers[1].number + 1);

    Ok(())
}



/// Tests that the newHeads subscription does not include the pending block. 
/// This test should be deleted if we decide to support pending blocks in the newHeads subscription again.
#[tokio::test(flavor = "multi_thread")]
async fn ws_subscribe_new_heads_does_not_include_pending() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_ws_client(rollup.http_addr).await;
    let mut subscription = client.subscribe_blocks().await?;
    while let Ok(header) = subscription.try_recv() {
        println!("Received header before starting test: {}", header.number);
    }

    rollup.wait_for_next_blocks(2).await;
    // Send a transaction to ensure a pending block is created
    let tx = TransactionRequest::default().with_to(Address::ZERO);
    let _pending = client.send_transaction(tx).await?;

    let header_1 = subscription.try_recv().unwrap();
    let header_2 = subscription.try_recv().unwrap();
    let pending_block = client.get_block_by_number(BlockNumberOrTag::Pending).await?.expect("Pending block should be available");
    assert_eq!(pending_block.number(), header_2.number + 1);
    assert!(subscription.try_recv().is_err(), "Pending is not supported");

    assert_eq!(header_1.number + 1, header_2.number);

    Ok(())
}
