use alloy_primitives::{Address, TxKind, U256, U64};
use alloy_rpc_types_eth::TransactionRequest;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde::de::DeserializeOwned;
use sov_demo_rollup::MockDemoRollup;
use sov_eth_client::SimpleStorageClient;
use sov_modules_api::execution_mode::Native;
use sov_test_utils::test_rollup::TestRollup;

use crate::evm::evm_test_helper::{
    create_simple_storage_client, setup_test_rollup, EVM_EXTENSION, FEE_CAP_TOO_LOW_ERROR,
    INSUFFICIENT_FUNDS_ERROR, MAX_FEE_PER_GAS, SENDER_PRIV_KEY,
};

const GAS_LIMIT: u64 = 21_000;
const GAS_REQUIRED_EXCEEDS_ALLOWANCE_ERROR: &str = "gas required exceeds allowance";
const CONFLICTING_FEE_FIELDS_ERROR: &str =
    "both gasPrice and (maxFeePerGas or maxPriorityFeePerGas) specified";
const TIP_ABOVE_FEE_CAP_ERROR: &str = "max priority fee per gas higher than max fee per gas";

fn unfunded_caller() -> Address {
    Address::from([0x11; 20])
}

/// Returns the rollup handle alongside the client — the rollup must stay alive for the client to function.
async fn setup_client() -> (TestRollup<MockDemoRollup<Native>>, SimpleStorageClient) {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    (rollup, client)
}

async fn assert_unfunded_caller(
    client: &SimpleStorageClient,
    caller: Address,
) -> anyhow::Result<()> {
    let balance: U256 = client
        .ws
        .request("eth_getBalance", rpc_params![caller, "latest"])
        .await?;
    assert_eq!(
        balance,
        U256::ZERO,
        "test precondition failed: caller must start with zero balance"
    );
    Ok(())
}

fn base_request(caller: Address) -> TransactionRequest {
    TransactionRequest {
        from: Some(caller),
        to: Some(TxKind::Call(Address::ZERO)),
        gas: Some(GAS_LIMIT),
        value: Some(U256::ZERO),
        ..Default::default()
    }
}

async fn current_base_fee(client: &SimpleStorageClient) -> anyhow::Result<u128> {
    let base_fee: U256 = client.ws.request("eth_gasPrice", rpc_params![]).await?;
    Ok(base_fee.to::<u128>())
}

async fn assert_rpc_rejects<T: DeserializeOwned + std::fmt::Debug>(
    client: &SimpleStorageClient,
    method: &str,
    request: &TransactionRequest,
    expected_error_substring: &str,
) {
    let result: Result<T, _> = client
        .ws
        .request(method, rpc_params![request, "latest"])
        .await;
    let err = result.expect_err(&format!("{method} should reject this request"));
    let err_msg = err.to_string();
    assert!(
        err_msg.contains(expected_error_substring),
        "unexpected error for {method}: {err_msg}"
    );
}

async fn assert_rpc_succeeds<T: DeserializeOwned>(
    client: &SimpleStorageClient,
    method: &str,
    request: &TransactionRequest,
) -> T {
    client
        .ws
        .request(method, rpc_params![request, "latest"])
        .await
        .expect("RPC call must succeed")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO: re-enable with paymaster-aware balance check"]
async fn eth_call_rejects_unfunded_caller_with_gas_price() -> anyhow::Result<()> {
    let (_rollup, client) = setup_client().await;

    let caller = unfunded_caller();
    assert_unfunded_caller(&client, caller).await?;

    let request = TransactionRequest {
        gas_price: Some(MAX_FEE_PER_GAS),
        ..base_request(caller)
    };
    assert_rpc_rejects::<String>(&client, "eth_call", &request, INSUFFICIENT_FUNDS_ERROR).await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO: re-enable with paymaster-aware balance check"]
async fn eth_estimate_gas_rejects_unfunded_caller_with_omitted_gas_and_fee() -> anyhow::Result<()> {
    let (_rollup, client) = setup_client().await;

    let caller = unfunded_caller();
    assert_unfunded_caller(&client, caller).await?;

    let request = TransactionRequest {
        gas: None,
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(1),
        ..base_request(caller)
    };
    assert_rpc_rejects::<U64>(
        &client,
        "eth_estimateGas",
        &request,
        GAS_REQUIRED_EXCEEDS_ALLOWANCE_ERROR,
    )
    .await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO: re-enable with paymaster-aware balance check"]
async fn eth_estimate_gas_rejects_unfunded_caller_with_omitted_gas_and_gas_price(
) -> anyhow::Result<()> {
    let (_rollup, client) = setup_client().await;

    let caller = unfunded_caller();
    assert_unfunded_caller(&client, caller).await?;

    let request = TransactionRequest {
        gas: None,
        gas_price: Some(MAX_FEE_PER_GAS),
        ..base_request(caller)
    };
    assert_rpc_rejects::<U64>(
        &client,
        "eth_estimateGas",
        &request,
        GAS_REQUIRED_EXCEEDS_ALLOWANCE_ERROR,
    )
    .await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO: re-enable with paymaster-aware balance check"]
async fn eth_estimate_gas_rejects_missing_from_with_gas_price() -> anyhow::Result<()> {
    let (_rollup, client) = setup_client().await;
    assert_unfunded_caller(&client, Address::ZERO).await?;

    let request = TransactionRequest {
        to: Some(TxKind::Call(Address::ZERO)),
        gas_price: Some(MAX_FEE_PER_GAS),
        gas: Some(GAS_LIMIT),
        value: Some(U256::ZERO),
        ..Default::default()
    };
    assert_rpc_rejects::<U64>(
        &client,
        "eth_estimateGas",
        &request,
        INSUFFICIENT_FUNDS_ERROR,
    )
    .await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO: re-enable with paymaster-aware balance check"]
async fn eth_estimate_gas_rejects_missing_from_with_omitted_gas_and_gas_price() -> anyhow::Result<()>
{
    let (_rollup, client) = setup_client().await;
    assert_unfunded_caller(&client, Address::ZERO).await?;

    let request = TransactionRequest {
        to: Some(TxKind::Call(Address::ZERO)),
        gas_price: Some(MAX_FEE_PER_GAS),
        value: Some(U256::ZERO),
        ..Default::default()
    };
    assert_rpc_rejects::<U64>(
        &client,
        "eth_estimateGas",
        &request,
        GAS_REQUIRED_EXCEEDS_ALLOWANCE_ERROR,
    )
    .await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO: re-enable with paymaster-aware balance check"]
async fn eth_estimate_gas_rejects_unfunded_caller_with_max_fee_per_gas() -> anyhow::Result<()> {
    let (_rollup, client) = setup_client().await;

    let caller = unfunded_caller();
    assert_unfunded_caller(&client, caller).await?;

    let request = TransactionRequest {
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(1),
        ..base_request(caller)
    };
    assert_rpc_rejects::<U64>(
        &client,
        "eth_estimateGas",
        &request,
        INSUFFICIENT_FUNDS_ERROR,
    )
    .await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_call_rejects_below_base_fee_with_max_fee_per_gas() -> anyhow::Result<()> {
    let (_rollup, client) = setup_client().await;
    let caller = client.address();
    let base_fee = current_base_fee(&client).await?;
    assert!(base_fee > 0, "base fee should be non-zero for this test");

    let request = TransactionRequest {
        max_fee_per_gas: Some(0),
        max_priority_fee_per_gas: Some(0),
        ..base_request(caller)
    };
    assert_rpc_rejects::<String>(
        &client,
        "eth_call",
        &request,
        "max fee per gas less than block base fee",
    )
    .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_create_access_list_accepts_below_base_fee_with_max_fee_per_gas() -> anyhow::Result<()>
{
    let (_rollup, client) = setup_client().await;
    let caller = client.address();
    let base_fee = current_base_fee(&client).await?;
    assert!(base_fee > 0, "base fee should be non-zero for this test");

    let request = TransactionRequest {
        max_fee_per_gas: Some(0),
        max_priority_fee_per_gas: Some(0),
        ..base_request(caller)
    };
    let result: serde_json::Value =
        assert_rpc_succeeds(&client, "eth_createAccessList", &request).await;
    assert!(
        result.get("accessList").is_some(),
        "missing accessList: {result}"
    );
    assert!(result.get("gasUsed").is_some(), "missing gasUsed: {result}");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_estimate_gas_rejects_below_base_fee_with_max_fee_per_gas() -> anyhow::Result<()> {
    let (_rollup, client) = setup_client().await;
    let caller = client.address();
    let base_fee = current_base_fee(&client).await?;
    assert!(base_fee > 0, "base fee should be non-zero for this test");

    let request = TransactionRequest {
        max_fee_per_gas: Some(0),
        max_priority_fee_per_gas: Some(0),
        ..base_request(caller)
    };
    assert_rpc_rejects::<U64>(&client, "eth_estimateGas", &request, FEE_CAP_TOO_LOW_ERROR).await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_create_access_list_rejects_conflicting_fee_fields() -> anyhow::Result<()> {
    let (_rollup, client) = setup_client().await;
    let caller = client.address();

    let request = TransactionRequest {
        gas_price: Some(MAX_FEE_PER_GAS),
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        ..base_request(caller)
    };
    assert_rpc_rejects::<serde_json::Value>(
        &client,
        "eth_createAccessList",
        &request,
        CONFLICTING_FEE_FIELDS_ERROR,
    )
    .await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_create_access_list_rejects_tip_above_fee_cap() -> anyhow::Result<()> {
    let (_rollup, client) = setup_client().await;
    let caller = client.address();

    let request = TransactionRequest {
        max_fee_per_gas: Some(1),
        max_priority_fee_per_gas: Some(2),
        ..base_request(caller)
    };
    assert_rpc_rejects::<serde_json::Value>(
        &client,
        "eth_createAccessList",
        &request,
        TIP_ABOVE_FEE_CAP_ERROR,
    )
    .await;

    Ok(())
}
