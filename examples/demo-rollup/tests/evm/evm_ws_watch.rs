use std::time::Duration;

use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use alloy::transports::RpcError;
use alloy_primitives::{Address, B256};
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
    let receipt = pending.get_receipt().await?;

    assert_eq!(receipt.transaction_hash, hash);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ws_subscribe_new_heads() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_ws_client(rollup.http_addr).await;

    let err = client.subscribe_blocks().await.unwrap_err();
    let RpcError::ErrorResp(payload) = err else {
        panic!("Expected subscription error")
    };
    let data = payload.data.unwrap();
    assert_eq!(data.get(), "\"Only LOG subscriptions are supported\"");

    Ok(())
}
