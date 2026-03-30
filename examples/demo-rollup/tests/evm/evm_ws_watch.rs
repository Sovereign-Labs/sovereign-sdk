use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use alloy_primitives::{Address, B256};
use futures::StreamExt;
use std::time::Duration;
use tokio::time::timeout;

use crate::evm::evm_test_helper::alloy_ws_client;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::{setup_test_rollup, setup_test_rollup_with_ideal_lag};

#[tokio::test(flavor = "multi_thread")]
async fn ws_watch_returns_receipt() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
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
    rollup.wait_for_rollup_height_advance_by(1).await;
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
    rollup.wait_for_rollup_height_advance_by(3).await;

    let headers: Vec<_> = subscription.into_stream().take(3).collect().await;

    assert_eq!(headers.len(), 3);
    assert_eq!(headers[1].number, headers[0].number + 1);
    assert_eq!(headers[2].number, headers[1].number + 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ws_subscribe_new_heads_sizes() -> anyhow::Result<()> {
    let rollup = setup_test_rollup_with_ideal_lag(0, EVM_EXTENSION, 0).await;
    let client = alloy_ws_client(rollup.http_addr).await;
    let mut subscription = client.subscribe_blocks().await?;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let header = subscription.recv().await?;
    assert_eq!(header.number, 2);
    // Block size is 512 bytes with gas_limit = 100_000_000_000 (5-byte RLP encoding).
    // The cached body RLP omits withdrawals while withdrawals are unavailable.
    assert_eq!(header.size.unwrap().to::<u64>(), 512);

    Ok(())
}
