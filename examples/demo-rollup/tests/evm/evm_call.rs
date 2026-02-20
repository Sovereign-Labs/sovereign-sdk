use std::net::SocketAddr;

use reqwest::Client;
use serde_json::{json, Value};

use crate::evm::evm_test_helper::{setup_with_simple_storage, EVM_EXTENSION};

const INVALID_PARAMS_CODE: i64 = -32602;
const REVERT_ERROR_CODE: i64 = 3;

async fn rpc_call(
    client: &Client,
    http_addr: SocketAddr,
    method: &str,
    params: Value,
) -> anyhow::Result<Value> {
    Ok(client
        .post(format!("http://{http_addr}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        }))
        .send()
        .await?
        .json::<Value>()
        .await?)
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_call_existing_contract_returns_non_empty_data() -> anyhow::Result<()> {
    let (rollup, evm_client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract_address = evm_client.alloy_deploy_contract().await;

    let input = format!("0x{}", hex::encode(evm_client.contract.get()));
    let from = format!("{:#x}", evm_client.address());
    let to = format!("{contract_address:#x}");

    let client = Client::new();
    let response = rpc_call(
        &client,
        rollup.http_addr,
        "eth_call",
        json!([{
            "from": from,
            "to": to,
            "gas": "0x7a120",
            "input": input
        }, "latest"]),
    )
    .await?;

    let result = response
        .get("result")
        .and_then(Value::as_str)
        .expect("eth_call should return result");

    assert_ne!(result, "0x", "eth_call should return ABI-encoded data");
    assert_eq!(
        result.len(),
        66,
        "expected 32-byte ABI return value (0x + 64 hex chars)"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_call_revert_returns_rpc_error() -> anyhow::Result<()> {
    let (rollup, evm_client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract_address = evm_client.alloy_deploy_contract().await;

    let input = format!("0x{}", hex::encode(evm_client.contract.always_revert()));
    let from = format!("{:#x}", evm_client.address());
    let to = format!("{contract_address:#x}");

    let client = Client::new();
    let response = rpc_call(
        &client,
        rollup.http_addr,
        "eth_call",
        json!([{
            "from": from,
            "to": to,
            "gas": "0x7a120",
            "input": input
        }, "latest"]),
    )
    .await?;

    let error = response
        .get("error")
        .expect("reverting eth_call should return an error object");
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .expect("error.code should be present");
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();

    assert_eq!(
        code, REVERT_ERROR_CODE,
        "expected geth-compatible revert error code"
    );
    assert!(
        message.contains("execution reverted"),
        "expected revert message, got: {message}"
    );

    let data = error
        .get("data")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        data.starts_with("0x"),
        "revert error should include hex-encoded revert data"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_call_invalid_params_returns_invalid_params_code() -> anyhow::Result<()> {
    let (rollup, _, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let client = Client::new();
    let response = rpc_call(
        &client,
        rollup.http_addr,
        "eth_call",
        json!([{
            "from": "0x0000000000000000000000000000000000000000",
            "to": "0x0000000000000000000000000000000000000000",
            "input": "not-hex"
        }, "latest"]),
    )
    .await?;

    let error = response
        .get("error")
        .expect("invalid call data should return error object");
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .expect("error.code should be present");
    assert_eq!(code, INVALID_PARAMS_CODE);

    Ok(())
}
