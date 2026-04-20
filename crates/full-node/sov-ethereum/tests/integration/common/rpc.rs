use std::net::SocketAddr;

use alloy_primitives::B256;
use reqwest::Client;
use serde_json::{json, Value};

pub async fn rpc_call(
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

pub fn hex_u64(value: u64) -> String {
    format!("0x{value:x}")
}

pub fn hex_word_u64(value: u64) -> String {
    format!("0x{value:064x}")
}

pub fn hex_u128(value: u128) -> String {
    format!("0x{value:x}")
}

pub fn parse_hex_u64(value: &str) -> u64 {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.is_empty() {
        return 0;
    }
    u64::from_str_radix(hex, 16).expect("valid u64 hex quantity")
}

pub fn parse_hex_u128(value: &str) -> u128 {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.is_empty() {
        return 0;
    }
    u128::from_str_radix(hex, 16).expect("valid u128 hex quantity")
}

pub fn rpc_result_hex(response: &Value) -> String {
    response
        .get("result")
        .and_then(Value::as_str)
        .expect("result should be a hex string")
        .to_string()
}

pub fn rpc_result_str<'a>(response: &'a Value, method: &str) -> &'a str {
    response
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{method} should return result"))
}

pub fn rpc_error_object<'a>(response: &'a Value, method: &str) -> &'a Value {
    response
        .get("error")
        .unwrap_or_else(|| panic!("{method} should return an error object"))
}

pub fn rpc_error_code(error: &Value) -> i64 {
    error
        .get("code")
        .and_then(Value::as_i64)
        .expect("error.code should be present")
}

pub fn rpc_error_message(error: &Value) -> &str {
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

pub fn rpc_error_data_str(error: &Value) -> Option<&str> {
    error.get("data").and_then(Value::as_str)
}

pub fn rpc_error_code_from_response(response: &Value, method: &str) -> i64 {
    rpc_error_code(rpc_error_object(response, method))
}

pub fn assert_invalid_params(response: &Value) {
    let error = rpc_error_object(response, "assert_invalid_params");
    assert_eq!(
        rpc_error_code(error),
        jsonrpsee::types::error::INVALID_PARAMS_CODE as i64,
        "expected JSON-RPC invalid params code"
    );
}

pub fn eth_call_params(from: &str, to: &str, input: &str, block_tag: &str) -> Value {
    json!([{
        "from": from,
        "to": to,
        "gas": "0x7a120",
        "input": input
    }, block_tag])
}

pub fn hash_selector(hash: B256) -> Value {
    json!({
        "blockHash": format!("{:#x}", hash),
        "requireCanonical": true
    })
}
