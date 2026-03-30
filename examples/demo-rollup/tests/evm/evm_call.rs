use reqwest::Client;
use serde_json::json;

use crate::evm::evm_test_helper::{
    eth_call_params, rpc_call, rpc_error_code, rpc_error_data_str, rpc_error_message,
    rpc_error_object, rpc_result_str, setup_with_simple_storage, EVM_EXTENSION,
};

const REVERT_ERROR_CODE: i64 = 3;

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
        eth_call_params(&from, &to, &input, "latest"),
    )
    .await?;

    let result = rpc_result_str(&response, "eth_call");

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
        eth_call_params(&from, &to, &input, "latest"),
    )
    .await?;

    let error = rpc_error_object(&response, "eth_call");
    let code = rpc_error_code(error);
    let message = rpc_error_message(error);

    assert_eq!(
        code, REVERT_ERROR_CODE,
        "expected geth-compatible revert error code"
    );
    assert!(
        message.contains("execution reverted"),
        "expected revert message, got: {message}"
    );

    let data = rpc_error_data_str(error).unwrap_or_default();
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

    let error = rpc_error_object(&response, "eth_call");
    let code = rpc_error_code(error);
    assert_eq!(code, jsonrpsee::types::error::INVALID_PARAMS_CODE as i64);

    Ok(())
}
