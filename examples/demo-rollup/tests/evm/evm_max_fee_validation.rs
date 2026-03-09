use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, U256};
use alloy_provider::DynProvider;
use alloy_rpc_types_eth::TransactionInput;
use demo_stf::runtime::{Runtime, RuntimeCall};
use sov_demo_rollup::MockDemoRollup;
use sov_evm::{CallMessage, EvmRuntimeConfigUpdate};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::transaction::Transaction;
use sov_test_utils::default_test_signed_transaction_with_nonce;
use sov_test_utils::test_rollup::{read_private_key, TestRollup};

use crate::evm::evm_test_helper::{
    alloy_client_with_signer, setup_test_rollup, EVM_EXTENSION, SENDER_PRIV_KEY,
};
use crate::test_helpers::{DemoRollupSpec, CHAIN_HASH};

const GAS_LIMIT: u64 = 100_000;

/// Sets up a test rollup and client, waiting for initial blocks and pausing batches
async fn setup() -> (TestRollup<MockDemoRollup<Native>>, DynProvider) {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client_with_signer(rollup.http_addr, SENDER_PRIV_KEY);
    rollup.wait_for_sequencer_ready().await.unwrap();
    (rollup, client)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_disable_max_fee_check() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_EVM_MAX_FEE_CHECK_HEIGHT", "0");
    let (rollup, client) = setup().await;

    let low_fee = 1;
    let high_fee = 1_000_000;

    // Verify low fee fails and high fee passes with max fee check enabled
    send_tx_expect_failure(
        &client,
        low_fee,
        "Low fee should fail before disable_max_fee_check message.",
    )
    .await;

    send_tx_expect_success(
        &client,
        high_fee,
        "High fee should pass before disable_max_fee_check message.",
    )
    .await;

    // Disable the max fee check via admin config update
    disable_max_fee_check(&rollup).await?;

    // Now both low and high fee transactions should succeed
    send_tx_expect_success(
        &client,
        low_fee,
        "Low fee should pass after disable_max_fee_check message.",
    )
    .await;

    send_tx_expect_success(
        &client,
        high_fee,
        "High fee should pass after disable_max_fee_check message.",
    )
    .await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_max_fee_check_height_is_respected() -> anyhow::Result<()> {
    // Set a higher threshold so we can test before/after behavior
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_EVM_MAX_FEE_CHECK_HEIGHT", "15");

    let (rollup, client) = setup().await;

    let low_fee = 1;
    let high_fee = 1_000_000;

    // Before threshold: both low and high fee should pass
    send_tx_expect_success(
        &client,
        low_fee,
        "Low fee should pass before height threshold.",
    )
    .await;
    send_tx_expect_success(
        &client,
        high_fee,
        "High fee should pass before height threshold.",
    )
    .await;

    // Advance past the threshold (height 15)
    rollup.wait_for_rollup_height_advance_by(20).await;

    // After threshold: low fee should fail, high fee should pass
    send_tx_expect_failure(
        &client,
        low_fee,
        "Low fee should fail after height threshold.",
    )
    .await;
    send_tx_expect_success(
        &client,
        high_fee,
        "High fee should pass after height threshold.",
    )
    .await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_big_call_data() {
    let (rollup, client) = setup().await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let mut tx = TransactionRequest::default().with_to(Address::ZERO);

    tx.input = TransactionInput {
        input: Some(vec![1; 1_000_000].into()),
        data: None,
    };

    let pending = client.send_transaction(tx).await.unwrap();
    _ = pending.watch().await.unwrap();
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

/// Sends a transaction and asserts it succeeds
async fn send_tx_expect_success(client: &DynProvider, fee: u128, msg: &str) {
    let tx_request = create_tx_request(client, fee).await.unwrap();
    let pending_tx = client.send_transaction(tx_request).await.unwrap();
    let receipt = pending_tx.get_receipt().await.unwrap();
    assert!(receipt.status(), "{msg}");
}

/// Sends a transaction and asserts it fails for the expected low fee-cap reason.
async fn send_tx_expect_failure(client: &DynProvider, fee: u128, msg: &str) {
    let tx_request = create_tx_request(client, fee).await.unwrap();
    let err = client.send_transaction(tx_request).await.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("max fee per gas less than block base fee")
            || err_msg.contains("Insufficient max_fee_per_gas"),
        "{msg}: got {err_msg}"
    );
}

/// Sends an empty EvmRuntimeConfigUpdate to disable the max fee check.
async fn disable_max_fee_check(rollup: &TestRollup<MockDemoRollup<Native>>) -> anyhow::Result<()> {
    let key_and_address = read_private_key::<DemoRollupSpec>("token_deployer_private_key.json");
    let admin_key = key_and_address.private_key;

    let update = EvmRuntimeConfigUpdate::<DemoRollupSpec>::empty();
    let msg = RuntimeCall::<DemoRollupSpec>::Evm(CallMessage::UpdateRuntimeConfig(update));

    let tx: Transaction<Runtime<DemoRollupSpec>, DemoRollupSpec> =
        default_test_signed_transaction_with_nonce(&admin_key, &msg, 0, &CHAIN_HASH);

    rollup
        .client
        .client
        .send_tx_to_sequencer_with_retry(&tx)
        .await?;

    Ok(())
}
