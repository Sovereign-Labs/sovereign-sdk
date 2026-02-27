use std::net::SocketAddr;

use alloy::consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy::eips::Encodable2718;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy_primitives::{hex, Address, TxKind, B256, U256, U64};
use alloy_provider::Provider;
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use sov_eth_client::SimpleStorageClient;
use sov_evm_test_utils::SimpleStorage;

use crate::evm::evm_test_helper::{
    alloy_client, alloy_ws_client, create_simple_storage_client, deploy_contract_check,
    setup_test_rollup, setup_with_simple_storage, EVM_EXTENSION, SENDER_PRIV_KEY,
};

const MAX_FEE_PER_GAS: u128 = 1_000_000_000;
const MAX_PRIORITY_FEE_PER_GAS: u128 = 1;

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

fn hex_u64(value: u64) -> String {
    format!("0x{value:x}")
}

fn hex_word_u64(value: u64) -> String {
    format!("0x{value:064x}")
}

fn error_code(response: &Value) -> i64 {
    response["error"]["code"]
        .as_i64()
        .expect("error.code should be present")
}

fn rpc_result_hex(response: &Value) -> String {
    response["result"]
        .as_str()
        .expect("result should be a hex string")
        .to_string()
}

async fn tx_count(
    client: &SimpleStorageClient,
    address: Address,
    block: impl Serialize,
) -> anyhow::Result<u64> {
    let count: U64 = client
        .ws
        .request("eth_getTransactionCount", rpc_params![address, block])
        .await?;
    Ok(count.to::<u64>())
}

async fn raw_signed_transfer(
    signer: &PrivateKeySigner,
    chain_id: u64,
    nonce: u64,
    gas_limit: u64,
    to: Address,
    value: U256,
) -> anyhow::Result<String> {
    let tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        to: TxKind::Call(to),
        value,
        input: Default::default(),
        access_list: Default::default(),
    };
    let sig = signer.sign_hash(&tx.signature_hash()).await?;
    let envelope = TxEnvelope::Eip1559(tx.into_signed(sig));
    Ok(format!("0x{}", hex::encode(envelope.encoded_2718())))
}

// RPC-001
#[tokio::test(flavor = "multi_thread")]
async fn rpc_001_block_number_matches_latest_with_pending_head() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let provider = alloy_client(rollup.http_addr);
    rollup.pause_preferred_batches().await;

    let tx_hash = client.send_eth(Address::ZERO, U256::from(1)).await;
    client.wait_for_receipt(tx_hash).await;

    let block_number = provider.get_block_number().await?;
    let latest = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .expect("latest should exist")
        .header
        .number;
    assert_eq!(block_number, latest);

    Ok(())
}

// RPC-002
#[tokio::test(flavor = "multi_thread")]
async fn rpc_002_block_pinned_nonce_excludes_pending_tx() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let sender = client.address();

    let finalized = client
        .eth_get_block_by_number(Some("finalized".to_string()))
        .await;
    let sealed_number = finalized.header.number;
    let sealed_selector = hex_u64(sealed_number);
    let before_sealed = tx_count(&client, sender, sealed_selector.clone()).await?;
    let before_latest = tx_count(&client, sender, "latest").await?;
    assert_eq!(before_sealed, before_latest);

    rollup.pause_preferred_batches().await;
    let tx_hash = client.send_eth(Address::ZERO, U256::from(7)).await;
    client.wait_for_receipt(tx_hash).await;

    let after_sealed = tx_count(&client, sender, sealed_selector).await?;
    let after_latest = tx_count(&client, sender, "latest").await?;
    let after_pending = tx_count(&client, sender, "pending").await?;

    assert_eq!(after_sealed, before_sealed);
    assert_eq!(after_latest, before_latest + 1);
    assert_eq!(after_pending, before_latest + 1);

    Ok(())
}

// RPC-003
#[tokio::test(flavor = "multi_thread")]
async fn rpc_003_estimate_gas_uses_account_nonce_when_nonce_omitted() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let provider = alloy_client(rollup.http_addr);
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let sender = client.address();

    // Consume nonce 0.
    let first_tx = client.send_eth(Address::ZERO, U256::from(1)).await;
    client.wait_for_receipt(first_tx).await;

    let deploy = SimpleStorage::deploy_builder(provider.clone());
    let deploy_request = deploy.into_transaction_request();
    let bytecode = deploy_request.input.into_input();

    let request_omitted_nonce = json!({
        "from": sender,
        "data": bytecode,
        "maxFeePerGas": hex_u64(MAX_FEE_PER_GAS as u64),
        "maxPriorityFeePerGas": "0x1"
    });
    let estimate_omitted_nonce: U64 = client
        .ws
        .request(
            "eth_estimateGas",
            rpc_params![request_omitted_nonce.clone(), "latest"],
        )
        .await?;
    assert!(estimate_omitted_nonce.to::<u64>() > 0);

    let request_nonce_zero = json!({
        "from": sender,
        "data": bytecode,
        "maxFeePerGas": hex_u64(MAX_FEE_PER_GAS as u64),
        "maxPriorityFeePerGas": "0x1",
        "nonce": "0x0"
    });
    let estimate_with_zero_nonce: Result<U64, _> = client
        .ws
        .request("eth_estimateGas", rpc_params![request_nonce_zero, "latest"])
        .await;
    let err = estimate_with_zero_nonce.expect_err("nonce=0 should be rejected after first tx");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("nonce too low")
            || err_msg.contains("already used")
            || err_msg.contains("nonce"),
        "unexpected error: {err_msg}"
    );

    Ok(())
}

// RPC-004
#[tokio::test(flavor = "multi_thread")]
async fn rpc_004_eth_call_applies_state_overrides() -> anyhow::Result<()> {
    let (_rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let set_tx_hash = client.set_value(contract, 1).await;
    client.wait_for_receipt(set_tx_hash).await;

    let get_tx = client.make_tx(Some(contract), Some(client.contract.get()));
    let baseline: B256 = client
        .ws
        .request("eth_call", rpc_params![&get_tx, "latest"])
        .await?;

    let overrides = json!({
        format!("{contract:#x}"): {
            "stateDiff": {
                hex_word_u64(0): hex_word_u64(42)
            }
        }
    });
    let overridden: B256 = client
        .ws
        .request("eth_call", rpc_params![&get_tx, "latest", overrides])
        .await?;

    assert_ne!(baseline, overridden);
    assert_eq!(
        U256::from_be_slice(overridden.as_slice()),
        U256::from(42u64)
    );

    Ok(())
}

// RPC-005
#[tokio::test(flavor = "multi_thread")]
async fn rpc_005_tx_rejection_should_use_standard_json_rpc_error_class() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let nonce = tx_count(&ws_client, signer.address(), "latest").await?;
    let chain_id: U64 = ws_client.ws.request("eth_chainId", rpc_params![]).await?;
    // Deliberately invalid gas limit to trigger deterministic tx rejection.
    let raw = raw_signed_transfer(
        &signer,
        chain_id.to::<u64>(),
        nonce,
        21_000,
        Address::repeat_byte(0x33),
        U256::from(1),
    )
    .await?;

    let http = Client::new();
    let response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendRawTransaction",
        json!([raw]),
    )
    .await?;
    assert!(
        response.get("error").is_some(),
        "expected rejected transaction error: {response}"
    );
    // Ethereum clients commonly classify malformed tx params as -32602.
    assert_eq!(error_code(&response), -32602);

    Ok(())
}

// RPC-006
#[tokio::test(flavor = "multi_thread")]
async fn rpc_006_get_balance_accepts_eip_1898_block_selector() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let address = client.address();
    rollup.wait_for_next_blocks(1).await;

    let provider = alloy_client(rollup.http_addr);
    let finalized = provider
        .get_block_by_number(BlockNumberOrTag::Finalized)
        .await?
        .expect("finalized should exist");
    let block_number = finalized.header.number;
    let block_hash = finalized.header.hash;

    let by_number: U256 = client
        .ws
        .request(
            "eth_getBalance",
            rpc_params![address, hex_u64(block_number)],
        )
        .await?;
    let by_hash: U256 = client
        .ws
        .request(
            "eth_getBalance",
            rpc_params![
                address,
                json!({
                    "blockHash": format!("{block_hash:#x}"),
                    "requireCanonical": true
                })
            ],
        )
        .await?;

    assert_eq!(by_number, by_hash);

    Ok(())
}

// RPC-007
#[tokio::test(flavor = "multi_thread")]
async fn rpc_007_web3_client_version_should_be_available() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let http = Client::new();

    let response = rpc_call(&http, rollup.http_addr, "web3_clientVersion", json!([])).await?;
    assert!(
        response.get("error").is_none(),
        "web3_clientVersion should be implemented: {response}"
    );
    assert!(response["result"].is_string());

    Ok(())
}

// RPC-008
#[tokio::test(flavor = "multi_thread")]
async fn rpc_008_receipt_fee_fields_match_balance_delta() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let sender = client.address();
    let recipient = Address::repeat_byte(0x77);
    let value = U256::from(1_000_000u64);

    let balance_before = client.eth_get_balance(sender).await;
    let tx_hash = client.send_eth(recipient, value).await;
    let receipt = client.wait_for_finalized_receipt(tx_hash).await;
    let balance_after = client.eth_get_balance(sender).await;

    let gas_cost = U256::from(receipt.gas_used) * U256::from(receipt.effective_gas_price);
    let expected_spent = value + gas_cost;
    let actual_spent = balance_before
        .checked_sub(balance_after)
        .expect("balance should decrease");

    assert_eq!(
        actual_spent, expected_spent,
        "receipt fee/value accounting should match sender balance delta"
    );

    Ok(())
}

// RPC-009
#[tokio::test(flavor = "multi_thread")]
async fn rpc_009_pending_trace_matches_original_tx_input() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    rollup.pause_preferred_batches().await;

    let tx1 = client.set_value(contract, 11).await;
    client.wait_for_receipt(tx1).await;
    let tx2 = client.set_value(contract, 22).await;
    client.wait_for_receipt(tx2).await;

    let tx1_json: Value = client
        .ws
        .request("eth_getTransactionByHash", rpc_params![tx1])
        .await?;
    let tx1_input = tx1_json["input"]
        .as_str()
        .expect("tx input should be present")
        .to_string();

    let trace: Value = client
        .ws
        .request(
            "debug_traceTransaction",
            rpc_params![tx1, json!({"tracer": "callTracer"})],
        )
        .await?;
    let trace_input = trace["input"]
        .as_str()
        .expect("trace input should be present")
        .to_string();
    assert_eq!(trace_input, tx1_input);
    assert!(
        trace.get("error").is_none() || trace["error"].is_null(),
        "trace should not contain execution error: {trace}"
    );

    Ok(())
}

// RPC-010
#[tokio::test(flavor = "multi_thread")]
async fn rpc_010_send_raw_transaction_sync_returns_receipt_under_preferred_sequencer(
) -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let nonce = tx_count(&client, signer.address(), "latest").await?;
    let chain_id: U64 = client.ws.request("eth_chainId", rpc_params![]).await?;

    let raw = raw_signed_transfer(
        &signer,
        chain_id.to::<u64>(),
        nonce,
        300_000,
        Address::repeat_byte(0x22),
        U256::from(1),
    )
    .await?;

    let http = Client::new();
    let response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendRawTransactionSync",
        json!([raw, 500]),
    )
    .await?;

    assert!(
        response.get("error").is_none(),
        "eth_sendRawTransactionSync should return receipt under preferred sequencer semantics: {response}"
    );
    assert!(
        response.get("result").is_some() && !response["result"].is_null(),
        "expected a non-null receipt result: {response}"
    );
    assert!(
        response["result"]["transactionHash"].is_string(),
        "receipt should include transactionHash: {response}"
    );
    assert!(
        response["result"]["blockNumber"].is_string(),
        "receipt should include blockNumber: {response}"
    );

    Ok(())
}

// RPC-011
#[tokio::test(flavor = "multi_thread")]
async fn rpc_011_pruned_log_range_should_not_use_custom_4444_code() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(2).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let log_tx = client.set_value(contract, 123).await;
    let log_receipt = client.wait_for_finalized_receipt(log_tx).await;
    let log_block = log_receipt
        .block_number
        .expect("log tx should be finalized and have block number");

    // Wait until the tx/log-bearing block is old enough that lookup can go through the pruned path.
    rollup.wait_for_next_blocks(45).await;

    let http = Client::new();
    let response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getLogs",
        json!([{
            "fromBlock": hex_u64(log_block),
            "toBlock": hex_u64(log_block)
        }]),
    )
    .await?;

    if response.get("error").is_some() {
        assert_eq!(
            error_code(&response),
            -32001,
            "pruned log range should use resource-not-found JSON-RPC class"
        );
    } else {
        assert!(
            response["result"].is_array(),
            "eth_getLogs should return either logs array or JSON-RPC error"
        );
    }

    Ok(())
}

// RPC-011 (trace path)
#[tokio::test(flavor = "multi_thread")]
async fn rpc_011_debug_trace_pruned_tx_should_not_use_custom_4444_code() -> anyhow::Result<()> {
    let override_key = "SOV_TEST_CONST_OVERRIDE_EVM_BLOCK_PRUNING_THRESHOLD";
    let previous_override = std::env::var(override_key).ok();
    std::env::set_var(override_key, "5");

    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(2).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    let tx_hash = client.send_eth(Address::ZERO, U256::from(1)).await;
    let _receipt = client.wait_for_finalized_receipt(tx_hash).await;

    let http = Client::new();
    let mut saw_pruned_error = false;

    for _ in 0..30 {
        rollup.wait_for_next_blocks(1).await;
        let response = rpc_call(
            &http,
            rollup.http_addr,
            "debug_traceTransaction",
            json!([tx_hash, {"tracer": "callTracer"}]),
        )
        .await?;

        if response.get("error").is_some() {
            saw_pruned_error = true;
            assert_eq!(
                error_code(&response),
                -32001,
                "pruned trace should use resource-not-found JSON-RPC class"
            );
            break;
        }
    }

    match previous_override {
        Some(value) => std::env::set_var(override_key, value),
        None => std::env::remove_var(override_key),
    }

    assert!(
        saw_pruned_error,
        "expected debug_traceTransaction to eventually hit pruned history path"
    );

    Ok(())
}

// RPC-012
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: eth_subscribe rejects block range options for logs"]
async fn rpc_012_log_subscription_should_accept_numeric_block_range() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let ws_client = alloy_ws_client(rollup.http_addr).await;
    let filter = Filter::new()
        .from_block(BlockNumberOrTag::Number(0))
        .to_block(BlockNumberOrTag::Latest);
    let sub = ws_client.subscribe_logs(&filter).await;
    assert!(
        sub.is_ok(),
        "logs subscriptions with block ranges should be accepted"
    );
    Ok(())
}

// RPC-013
#[tokio::test(flavor = "multi_thread")]
async fn rpc_013_create_prediction_matches_rpc_nonce_in_demo_harness() -> anyhow::Result<()> {
    let (_rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let sender = client.address();
    let nonce = tx_count(&client, sender, "latest").await?;

    let deploy_tx = client
        .deploy_contract()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let receipt = client.wait_for_receipt(deploy_tx).await;
    let deployed = receipt
        .contract_address
        .expect("deployment should produce address");
    let predicted = sender.create(nonce);

    assert_eq!(
        deployed, predicted,
        "in this harness CREATE prediction from eth_getTransactionCount nonce matches deployment"
    );

    Ok(())
}

// RPC-014
#[tokio::test(flavor = "multi_thread")]
async fn rpc_014_pending_semantics_matrix_is_internally_consistent() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let sender = client.address();

    let baseline_latest = tx_count(&client, sender, "latest").await?;
    let baseline_pending = tx_count(&client, sender, "pending").await?;
    assert_eq!(baseline_latest, baseline_pending);

    let finalized_block = client
        .eth_get_block_by_number(Some("finalized".to_string()))
        .await;
    let finalized_number = finalized_block.header.number;
    let finalized_selector = hex_u64(finalized_number);
    let baseline_finalized = tx_count(&client, sender, finalized_selector.clone()).await?;

    rollup.pause_preferred_batches().await;
    let tx_hash = client.send_eth(Address::ZERO, U256::from(3)).await;
    client.wait_for_receipt(tx_hash).await;

    let after_latest = tx_count(&client, sender, "latest").await?;
    let after_pending = tx_count(&client, sender, "pending").await?;
    let after_finalized = tx_count(&client, sender, finalized_selector).await?;

    assert_eq!(after_latest, baseline_latest + 1);
    assert_eq!(after_pending, baseline_pending + 1);
    assert_eq!(after_finalized, baseline_finalized);

    Ok(())
}

// RPC-015
#[tokio::test(flavor = "multi_thread")]
async fn rpc_015_estimate_gas_is_stable_for_identical_input() -> anyhow::Result<()> {
    let (_rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let set_tx = client.make_tx(Some(contract), Some(client.contract.set(0x1234)));

    let estimate1: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&set_tx, "latest"])
        .await?;
    let estimate2: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&set_tx, "latest"])
        .await?;

    assert!(estimate1.to::<u64>() > 0);
    assert_eq!(estimate1, estimate2);

    Ok(())
}

// RPC-016
#[tokio::test(flavor = "multi_thread")]
async fn rpc_016_eth_send_transaction_should_preserve_user_gas() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let from = signer.address();
    let requested_gas = 0x50_000u64;
    let requested_gas_hex = hex_u64(requested_gas);

    let http = Client::new();
    let send_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendTransaction",
        json!([{
            "from": from,
            "to": Address::ZERO,
            "gas": requested_gas_hex,
            "value": "0x0",
            "maxFeePerGas": hex_u64(MAX_FEE_PER_GAS as u64),
            "maxPriorityFeePerGas": "0x1"
        }]),
    )
    .await?;
    assert!(
        send_response.get("error").is_none(),
        "eth_sendTransaction should succeed in local mode: {send_response}"
    );
    let tx_hash = rpc_result_hex(&send_response);

    let tx_query_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getTransactionByHash",
        json!([tx_hash]),
    )
    .await?;
    assert!(
        tx_query_response.get("error").is_none(),
        "eth_getTransactionByHash should succeed: {tx_query_response}"
    );
    let tx_gas = tx_query_response["result"]["gas"]
        .as_str()
        .expect("transaction gas should be present")
        .to_string();

    assert_eq!(
        tx_gas, requested_gas_hex,
        "node should preserve user-provided gas limit in eth_sendTransaction"
    );

    Ok(())
}
