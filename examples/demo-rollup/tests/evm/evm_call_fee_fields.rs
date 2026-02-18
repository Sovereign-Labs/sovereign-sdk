use alloy_primitives::{Address, TxKind, U256, U64};
use alloy_rpc_types_eth::TransactionRequest;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;

use crate::evm::evm_test_helper::{
    create_simple_storage_client, setup_test_rollup, EVM_EXTENSION, SENDER_PRIV_KEY,
};

const GAS_LIMIT: u64 = 21_000;
const NONZERO_FEE_PER_GAS: u128 = 1_000_000_000;

fn unfunded_caller() -> Address {
    Address::from([0x11; 20])
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_call_rejects_unfunded_caller_with_gas_price() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    let caller = unfunded_caller();
    let balance: U256 = client
        .ws
        .request("eth_getBalance", rpc_params![caller, "latest"])
        .await?;
    assert_eq!(
        balance,
        U256::ZERO,
        "test precondition failed: caller must start with zero balance"
    );

    let request = TransactionRequest {
        from: Some(caller),
        to: Some(TxKind::Call(Address::ZERO)),
        gas_price: Some(NONZERO_FEE_PER_GAS),
        gas: Some(GAS_LIMIT),
        value: Some(U256::ZERO),
        ..Default::default()
    };

    let result: Result<String, _> = client
        .ws
        .request("eth_call", rpc_params![request, "latest"])
        .await;
    let err = result.expect_err("eth_call should fail for unfunded caller when gasPrice is set");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("insufficient funds for gas * price + value"),
        "unexpected error: {err_msg}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_estimate_gas_rejects_unfunded_caller_with_max_fee_per_gas() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    let caller = unfunded_caller();
    let balance: U256 = client
        .ws
        .request("eth_getBalance", rpc_params![caller, "latest"])
        .await?;
    assert_eq!(
        balance,
        U256::ZERO,
        "test precondition failed: caller must start with zero balance"
    );

    let request = TransactionRequest {
        from: Some(caller),
        to: Some(TxKind::Call(Address::ZERO)),
        max_fee_per_gas: Some(NONZERO_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(1),
        gas: Some(GAS_LIMIT),
        value: Some(U256::ZERO),
        ..Default::default()
    };

    let result: Result<U64, _> = client
        .ws
        .request("eth_estimateGas", rpc_params![request, "latest"])
        .await;
    let err = result
        .expect_err("eth_estimateGas should fail for unfunded caller when maxFeePerGas is set");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("insufficient funds for gas * price + value"),
        "unexpected error: {err_msg}"
    );

    Ok(())
}
