use alloy_primitives::{Address, U256};
use alloy_provider::{DynProvider, Provider};
use alloy_rpc_types_eth::TransactionRequest;
use alloy_signer::SignerSync;
use alloy_signers::local::PrivateKeySigner;
use sov_demo_rollup::MockDemoRollup;
use sov_modules_api::execution_mode::Native;
use sov_test_utils::test_rollup::TestRollup;

use crate::evm::evm_test_helper::{
    alloy_client_with_signer, setup_test_rollup, EVM_EXTENSION, SENDER_PRIV_KEY,
};

const GAS_LIMIT: u64 = 21000;

/// Sets up a test rollup and client, waiting for initial blocks and pausing batches
async fn setup_paused_rollup() -> (TestRollup<MockDemoRollup<Native>>, DynProvider) {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client_with_signer(rollup.http_addr, SENDER_PRIV_KEY);
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;
    (rollup, client)
}

/// Helper to create a simple ETH transfer transaction with the given max fee per gas
async fn create_tx_request(
    client: &impl Provider,
    max_fee_per_gas: u128,
) -> anyhow::Result<TransactionRequest> {
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let nonce = client.get_transaction_count(signer.address()).await?;

    Ok(TransactionRequest::default()
        .with_to(Address::ZERO)
        .with_nonce(nonce)
        .with_value(U256::ZERO)
        .with_max_fee_per_gas(max_fee_per_gas)
        .with_max_priority_fee_per_gas(0)
        .with_gas_limit(GAS_LIMIT))
}

/// Helper to send a transaction and wait for it to be mined
async fn send_and_mine_tx(
    rollup: &TestRollup<MockDemoRollup<Native>>,
    client: &DynProvider,
    max_fee_per_gas: u128,
) -> anyhow::Result<alloy_rpc_types_eth::TransactionReceipt> {
    let tx_request = create_tx_request(client, max_fee_per_gas).await?;
    let pending_tx = client.send_transaction(tx_request).await?;

    rollup.unpause_preferred_batches().await;
    rollup.wait_for_next_blocks(1).await;

    Ok(pending_tx.get_receipt().await?)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_insufficient_max_fee_per_gas() -> anyhow::Result<()> {
    let (_rollup, client) = setup_paused_rollup().await;

    let user_max_fee = 1u128;
    let tx_request = create_tx_request(&client, user_max_fee).await?;
    let result = client.send_transaction(tx_request).await;

    let err_msg = result.unwrap_err().to_string();
    assert_eq!(
        err_msg,
        "Insufficient max_fee_per_gas: user specified 1, but current base fee is 9"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sufficient_max_fee_per_gas() -> anyhow::Result<()> {
    let (rollup, client) = setup_paused_rollup().await;

    let receipt = send_and_mine_tx(&rollup, &client, 1_000_000_000).await?;
    assert!(receipt.status());

    Ok(())
}
