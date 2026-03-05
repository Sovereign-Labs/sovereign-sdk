use reqwest::Client;
use serde_json::{json, Value};

use crate::evm::evm_test_helper::{
    assert_invalid_params, rpc_call, setup_test_rollup, EVM_EXTENSION,
};

fn parse_hex_quantity(value: &str) -> u64 {
    let digits = value.strip_prefix("0x").unwrap_or(value);
    u64::from_str_radix(digits, 16).expect("valid hex quantity")
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_logs_reversed_block_range_returns_invalid_params() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = Client::new();

    let response = rpc_call(
        &client,
        rollup.http_addr,
        "eth_getLogs",
        json!([{
            "fromBlock": "0x29",
            "toBlock": "0x26"
        }]),
    )
    .await?;

    assert_invalid_params(&response);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_logs_future_block_range_returns_invalid_params() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = Client::new();

    let block_number_response =
        rpc_call(&client, rollup.http_addr, "eth_blockNumber", json!([])).await?;
    let latest_hex = block_number_response
        .get("result")
        .and_then(Value::as_str)
        .expect("eth_blockNumber should return a hex quantity");
    let latest = parse_hex_quantity(latest_hex);
    let future = latest.saturating_add(2);

    let response = rpc_call(
        &client,
        rollup.http_addr,
        "eth_getLogs",
        json!([{
            "fromBlock": format!("0x{latest:x}"),
            "toBlock": format!("0x{future:x}")
        }]),
    )
    .await?;

    assert_invalid_params(&response);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_logs_with_cursor_malformed_filter_returns_invalid_params() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = Client::new();

    let response = rpc_call(
        &client,
        rollup.http_addr,
        "eth_getLogsWithCursor",
        json!([{
            "cursor": 1
        }]),
    )
    .await?;

    assert_invalid_params(&response);
    Ok(())
}
