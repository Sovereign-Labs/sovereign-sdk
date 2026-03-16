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
    setup_test_rollup_with_paymaster, tx_count, EVM_EXTENSION, MAX_FEE_PER_GAS, SENDER_PRIV_KEY,
};

const GAS_LIMIT: u64 = 21_000;

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

#[derive(Debug, Clone)]
struct CombinedErrors {
    estimate_gas_error: String,
    call_error: String,
}

async fn get_consistent_response(
    client: &SimpleStorageClient,
    request: &TransactionRequest,
) -> Result<(), CombinedErrors> {
    let eth_estimate_gas_result: Result<U64, _> = client
        .ws
        .request("eth_estimateGas", rpc_params![request, "latest"])
        .await;

    let eth_call_result: Result<String, _> = client
        .ws
        .request("eth_call", rpc_params![request, "latest"])
        .await;

    match (eth_estimate_gas_result, eth_call_result) {
        (Ok(_), Ok(_)) => Ok(()),
        (Err(estimate_gas_error), Err(call_err)) => Err(CombinedErrors {
            estimate_gas_error: estimate_gas_error.to_string(),
            call_error: call_err.to_string(),
        }),
        (Ok(_), Err(call_err)) => {
            panic!("eth_estimateGas succeeded, but eth_call failed with {call_err}")
        }
        (Err(estimate_gas_err), Ok(_)) => {
            panic!("eth_call succeeded, but eth_estimateGas failed with {estimate_gas_err}")
        }
    }
}

async fn assert_simulation_rejects<T: DeserializeOwned + std::fmt::Debug>(
    client: &SimpleStorageClient,
    request: &TransactionRequest,
    expected_error_substring: &str,
) {
    let combined_err = get_consistent_response(client, request)
        .await
        .expect_err("Request should be rejected");

    assert!(
        combined_err.call_error.contains(expected_error_substring),
        "eth_call: expected error containing {expected_error_substring:?}, got: {}",
        combined_err.call_error,
    );
    assert!(
        combined_err
            .estimate_gas_error
            .contains(expected_error_substring),
        "eth_estimateGas: expected error containing {expected_error_substring:?}, got: {}",
        combined_err.estimate_gas_error
    );
}

async fn assert_simulation_succeeds(client: &SimpleStorageClient, request: &TransactionRequest) {
    get_consistent_response(client, request)
        .await
        .expect("Requests should be accepted");
}

// No fee fields → balance check skipped
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_without_fee_fields() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = base_request(addr);
    assert_simulation_succeeds(&client, &request).await;

    Ok(())
}

// gasPrice set → paymaster covers sender, simulation succeeds
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_with_gas_price() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas_price: Some(MAX_FEE_PER_GAS),
        ..base_request(addr)
    };
    assert_simulation_succeeds(&client, &request).await;

    Ok(())
}

// maxFeePerGas set → paymaster covers sender, simulation succeeds
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_with_max_fee_per_gas() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..base_request(addr)
    };
    assert_simulation_succeeds(&client, &request).await;

    Ok(())
}

// Both gasPrice and maxFeePerGas → conflicting fields error
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

// gasPrice set, gas omitted → paymaster covers sender, simulation succeeds
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_with_gas_price_gas_omitted() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas: None,
        gas_price: Some(MAX_FEE_PER_GAS),
        ..base_request(addr)
    };
    assert_simulation_succeeds(&client, &request).await;

    Ok(())
}

// maxFeePerGas set, gas omitted → paymaster covers sender, simulation succeeds
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_with_max_fee_per_gas_gas_omitted() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas: None,
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..base_request(addr)
    };
    assert_simulation_succeeds(&client, &request).await;

    Ok(())
}

// Test 7: Simulation succeeds and sendRawTransaction succeeds
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_send_raw_tx_succeeds() -> anyhow::Result<()> {
    let (rollup, client, addr) = setup_paymaster_client().await;

    // Simulation should succeed for paymaster-covered sender.
    let request = TransactionRequest {
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..base_request(addr)
    };
    assert_simulation_succeeds(&client, &request).await;

    // Use base_fee * 2 as a reasonable fee the paymaster's SOV bank can afford.
    let base_fee: U256 = client.ws.request("eth_gasPrice", rpc_params![]).await?;
    let base_fee_u128: u128 = base_fee.to::<u128>();
    assert!(base_fee_u128 > 0, "base fee should be non-zero");
    let reasonable_max_fee = base_fee_u128 * 2;

    // Send a real signed tx — sendRawTransaction goes through the mempool/STF
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
        reasonable_max_fee,
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
