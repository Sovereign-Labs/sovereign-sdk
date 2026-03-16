use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256, U64};
use alloy_rpc_types_eth::TransactionRequest;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::json;
use sov_demo_rollup::MockDemoRollup;
use sov_eth_client::SimpleStorageClient;
use sov_modules_api::execution_mode::Native;
use sov_test_utils::test_rollup::TestRollup;

use crate::evm::evm_test_helper::{
    create_simple_storage_client, raw_signed_eip1559, rpc_call, rpc_result_hex,
    setup_test_rollup_with_paymaster, tx_count, EVM_EXTENSION, INSUFFICIENT_FUNDS_ERROR,
    MAX_FEE_PER_GAS, SENDER_PRIV_KEY,
};

const GAS_LIMIT: u64 = 21_000;
const GAS_REQUIRED_EXCEEDS_ALLOWANCE_ERROR: &str = "gas required exceeds allowance";

/// Hardhat #4: 0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65
/// Not in any genesis → starts with zero EVM balance, but covered by the paymaster.
const PAYMASTER_SIGNER_PRIV_KEY: &str =
    "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a";

fn paymaster_address() -> Address {
    let signer: PrivateKeySigner = PAYMASTER_SIGNER_PRIV_KEY.parse().unwrap();
    signer.address()
}

async fn setup_paymaster_client() -> (
    TestRollup<MockDemoRollup<Native>>,
    SimpleStorageClient,
    Address,
) {
    let rollup = setup_test_rollup_with_paymaster(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    let addr = paymaster_address();
    let balance: U256 = client
        .ws
        .request("eth_getBalance", rpc_params![addr, "latest"])
        .await
        .unwrap();
    assert_eq!(
        balance,
        U256::ZERO,
        "test precondition failed: paymaster signer must start with zero EVM balance"
    );

    (rollup, client, addr)
}

fn base_request(from: Address) -> TransactionRequest {
    TransactionRequest {
        from: Some(from),
        to: Some(TxKind::Call(Address::ZERO)),
        gas: Some(GAS_LIMIT),
        value: Some(U256::ZERO),
        ..Default::default()
    }
}

async fn assert_simulation_rejects<T: DeserializeOwned + std::fmt::Debug>(
    client: &SimpleStorageClient,
    request: &TransactionRequest,
    expected_error_substring: &str,
) {
    for method in ["eth_call", "eth_estimateGas"] {
        let result: Result<T, _> = client
            .ws
            .request(method, rpc_params![request, "latest"])
            .await;
        let err = result.expect_err(&format!("{method} should reject this request"));
        let err_msg = err.to_string();
        assert!(
            err_msg.contains(expected_error_substring),
            "{method}: expected error containing {expected_error_substring:?}, got: {err_msg}"
        );
    }
}

async fn assert_simulation_succeeds(client: &SimpleStorageClient, request: &TransactionRequest) {
    let _: String = client
        .ws
        .request("eth_call", rpc_params![request, "latest"])
        .await
        .expect("eth_call must succeed");

    let _: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![request, "latest"])
        .await
        .expect("eth_estimateGas must succeed");
}

// ── Test 1: No fee fields → balance check skipped ──

#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_without_fee_fields() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = base_request(addr);
    assert_simulation_succeeds(&client, &request).await;

    Ok(())
}

// ── Test 2: gasPrice set → balance check fires, rejects zero-balance sender ──

#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_rejects_with_gas_price() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas_price: Some(MAX_FEE_PER_GAS),
        ..base_request(addr)
    };
    assert_simulation_rejects::<String>(&client, &request, INSUFFICIENT_FUNDS_ERROR).await;

    Ok(())
}

// ── Test 3: maxFeePerGas set → balance check fires, rejects zero-balance sender ──

#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_rejects_with_max_fee_per_gas() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..base_request(addr)
    };
    assert_simulation_rejects::<String>(&client, &request, INSUFFICIENT_FUNDS_ERROR).await;

    Ok(())
}

// ── Test 4: Both gasPrice and maxFeePerGas → conflicting fields error ──

#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_rejects_with_conflicting_fee_fields() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas_price: Some(MAX_FEE_PER_GAS),
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        ..base_request(addr)
    };
    assert_simulation_rejects::<String>(
        &client,
        &request,
        "both gasPrice and (maxFeePerGas or maxPriorityFeePerGas) specified",
    )
    .await;

    Ok(())
}

// ── Test 5: gasPrice set, gas omitted → affordable gas < 21000, exceeds allowance ──

#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_rejects_with_gas_price_gas_omitted() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas: None,
        gas_price: Some(MAX_FEE_PER_GAS),
        ..base_request(addr)
    };
    assert_simulation_rejects::<U64>(&client, &request, GAS_REQUIRED_EXCEEDS_ALLOWANCE_ERROR).await;

    Ok(())
}

// ── Test 6: maxFeePerGas set, gas omitted → affordable gas < 21000, exceeds allowance ──

#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_rejects_with_max_fee_per_gas_gas_omitted() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas: None,
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..base_request(addr)
    };
    assert_simulation_rejects::<U64>(&client, &request, GAS_REQUIRED_EXCEEDS_ALLOWANCE_ERROR).await;

    Ok(())
}

// ── Test 7: Simulation rejects but sendRawTransaction succeeds (key discrepancy) ──

#[tokio::test(flavor = "multi_thread")]
async fn paymaster_send_raw_tx_succeeds_despite_simulation_rejection() -> anyhow::Result<()> {
    let (rollup, client, addr) = setup_paymaster_client().await;

    // First, confirm simulation rejects with fee fields.
    let request = TransactionRequest {
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..base_request(addr)
    };
    let estimate_result: Result<U64, _> = client
        .ws
        .request("eth_estimateGas", rpc_params![&request, "latest"])
        .await;
    let err = estimate_result
        .expect_err("eth_estimateGas should reject zero-balance sender with fee fields");
    assert!(
        err.to_string().contains(INSUFFICIENT_FUNDS_ERROR),
        "expected insufficient funds error, got: {err}"
    );

    // Now send a real signed tx — sendRawTransaction goes through the mempool/STF
    // path which IS paymaster-aware.
    let paymaster_signer: PrivateKeySigner = PAYMASTER_SIGNER_PRIV_KEY.parse()?;
    let chain_id: U64 = client.ws.request("eth_chainId", rpc_params![]).await?;
    let nonce = tx_count(&client, addr, "latest").await?;

    let raw_tx = raw_signed_eip1559(
        &paymaster_signer,
        chain_id.to::<u64>(),
        nonce,
        GAS_LIMIT,
        TxKind::Call(Address::ZERO),
        U256::ZERO,
        Bytes::new(),
        MAX_FEE_PER_GAS,
        0,
    )
    .await?;

    let http = Client::new();
    let send_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendRawTransaction",
        json!([raw_tx]),
    )
    .await?;
    assert!(
        send_response.get("error").is_none(),
        "sendRawTransaction should succeed for paymaster-covered sender: {send_response}"
    );

    let tx_hash: B256 = rpc_result_hex(&send_response).parse()?;
    let receipt = client.wait_for_receipt(tx_hash).await;
    assert!(
        receipt.status(),
        "transaction should succeed when paymaster covers gas"
    );

    Ok(())
}
