use crate::evm::evm_test_helper::{
    create_simple_storage_client, deploy_contract_check, hex_u128, hex_u64, parse_hex_u128,
    parse_hex_u64, raw_signed_eip1559, rpc_call, rpc_error_code_from_response, rpc_error_message,
    rpc_error_object, rpc_result_hex, setup_test_rollup, setup_with_simple_storage, tx_count,
    EVM_EXTENSION, HIGH_MAX_FEE_PER_GAS, HIGH_PRIORITY_FEE_PER_GAS, SENDER_PRIV_KEY,
};
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256, U64};
use alloy_rpc_types_trace::geth::GethTrace;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use reqwest::Client;
use serde_json::json;

const DEFAULT_MAX_FEE_PER_GAS: u128 = 1_000_000_000;
const DEFAULT_MAX_PRIORITY_FEE_PER_GAS: u128 = 1;
const ETH_TX_GAS_CAP: u64 = 30_000_000;
const GAS_LEFT_CONTRACT_DEPLOY_CODE: &str = "0x6009600c60003960096000f35a60005260206000f3";
const GAS_LEFT_CONTRACT_RUNTIME_CODE: &[u8] =
    &[0x5a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3];
const AFFORDABILITY_SIGNER_PRIV_KEY: &str =
    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const AFFORDABILITY_BALANCE_MULTIPLIER: u64 = 100;
const INSUFFICIENT_FUNDS_FOR_GAS_ERROR: &str = "insufficient funds for gas * price + value";

// Overlap note: earlier low-fee-cap rejection coverage lives in
// `evm_call_fee_fields.rs::{eth_call_rejects_below_base_fee_with_max_fee_per_gas, eth_create_access_list_rejects_below_base_fee_with_max_fee_per_gas}`.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_001_estimate_send_max_fee_admission_consistency() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let nonce = tx_count(&ws_client, signer.address(), "latest").await?;
    let chain_id: U64 = ws_client.ws.request("eth_chainId", rpc_params![]).await?;
    let base_fee: U256 = ws_client.ws.request("eth_gasPrice", rpc_params![]).await?;
    let base_fee_u128: u128 = base_fee.to::<u128>();
    assert!(
        base_fee_u128 > 0,
        "base fee should be non-zero for this test"
    );

    let low_fee = base_fee_u128 - 1;
    let estimate_request = json!({
        "from": signer.address(),
        "to": Address::repeat_byte(0x11),
        "value": "0x0",
        "gas": "0x5208",
        "maxFeePerGas": hex_u128(low_fee),
        "maxPriorityFeePerGas": "0x0"
    });

    let http = Client::new();
    let estimate_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        json!([estimate_request, "latest"]),
    )
    .await?;

    let raw_tx = raw_signed_eip1559(
        &signer,
        chain_id.to::<u64>(),
        nonce,
        21_000,
        TxKind::Call(Address::repeat_byte(0x11)),
        U256::ZERO,
        Bytes::new(),
        low_fee,
        0,
    )
    .await?;

    let send_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendRawTransaction",
        json!([raw_tx]),
    )
    .await?;

    assert!(
        estimate_response.get("error").is_some(),
        "estimate should reject maxFeePerGas below base fee: {estimate_response}"
    );
    assert!(
        send_response.get("error").is_some(),
        "send should reject maxFeePerGas below base fee: {send_response}"
    );
    assert_eq!(
        rpc_error_code_from_response(&estimate_response, "eth_estimateGas"),
        rpc_error_code_from_response(&send_response, "eth_sendRawTransaction"),
        "estimate and send should reject with the same JSON-RPC error class"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
/// The signer is funded into the window
/// `gas_limit * rollup_gas_price < B < gas_limit * maxFeePerGas`, so both
/// `eth_estimateGas` and `eth_sendRawTransaction` must reject with an
/// insufficient-funds affordability error.
///
/// This is not an `eth_call` parity test. Local call/base-fee semantics live in
/// `evm_call_fee_fields.rs`.
async fn rpc2_002_estimate_send_affordability_consistency() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let affordability_signer: PrivateKeySigner = AFFORDABILITY_SIGNER_PRIV_KEY.parse()?;
    let affordability_address = affordability_signer.address();
    let chain_id: U64 = ws_client.ws.request("eth_chainId", rpc_params![]).await?;
    let gas_limit = 1_000_000u64;
    let recipient = Address::repeat_byte(0x22);
    let initial_rollup_gas_price = ws_client.eth_gas_price().await;
    let initial_send_floor = U256::from(gas_limit)
        .checked_mul(U256::from(initial_rollup_gas_price))
        .expect("send floor should fit in U256");
    let affordability_balance = initial_send_floor
        .checked_mul(U256::from(AFFORDABILITY_BALANCE_MULTIPLIER))
        .expect("affordability balance should fit in U256");
    let estimate_ceiling = U256::from(gas_limit)
        .checked_mul(U256::from(HIGH_MAX_FEE_PER_GAS))
        .expect("estimate ceiling should fit in U256");

    assert!(
        initial_rollup_gas_price > 0,
        "rollup gas price should be non-zero for this test"
    );
    assert_eq!(
        ws_client.eth_get_balance(affordability_address).await,
        U256::ZERO,
        "fresh affordability signer must start unfunded"
    );
    assert!(
        initial_send_floor < affordability_balance,
        "funding balance must exceed raw-send affordability floor"
    );
    assert!(
        affordability_balance < estimate_ceiling,
        "funding balance must remain below estimateGas affordability ceiling"
    );

    let funding_hash = ws_client
        .send_eth(affordability_address, affordability_balance)
        .await;
    let funding_receipt = ws_client.wait_for_receipt(funding_hash).await;
    assert!(funding_receipt.status(), "funding transfer should succeed");
    assert_eq!(
        ws_client.eth_get_balance(affordability_address).await,
        affordability_balance,
        "recipient balance should exactly match the funded affordability window"
    );

    let current_rollup_gas_price = ws_client.eth_gas_price().await;
    let current_send_floor = U256::from(gas_limit)
        .checked_mul(U256::from(current_rollup_gas_price))
        .expect("current send floor should fit in U256");
    assert!(
        current_send_floor < affordability_balance,
        "fresh signer must remain able to pay raw-send affordability floor after funding"
    );

    let nonce = tx_count(&ws_client, affordability_address, "latest").await?;
    assert_eq!(nonce, 0, "receiving funds should not change sender nonce");
    let estimate_request = json!({
        "from": affordability_address,
        "to": recipient,
        "value": "0x0",
        "gas": hex_u64(gas_limit),
        "maxFeePerGas": hex_u128(HIGH_MAX_FEE_PER_GAS),
        "maxPriorityFeePerGas": "0x0"
    });

    let http = Client::new();
    let estimate_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        json!([estimate_request, "latest"]),
    )
    .await?;

    let raw_tx = raw_signed_eip1559(
        &affordability_signer,
        chain_id.to::<u64>(),
        nonce,
        gas_limit,
        TxKind::Call(recipient),
        U256::ZERO,
        Bytes::new(),
        HIGH_MAX_FEE_PER_GAS,
        0,
    )
    .await?;

    let send_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendRawTransaction",
        json!([raw_tx]),
    )
    .await?;
    assert_eq!(
        rpc_error_code_from_response(&estimate_response, "eth_estimateGas"),
        -32003,
        "estimate should use transaction-rejected error class for insufficient funds"
    );
    assert!(
        rpc_error_message(rpc_error_object(&estimate_response, "eth_estimateGas"))
            .contains(INSUFFICIENT_FUNDS_FOR_GAS_ERROR),
        "estimate should report insufficient-funds affordability failure: {estimate_response}"
    );

    assert_eq!(
        rpc_error_code_from_response(&estimate_response, "eth_estimateGas"),
        rpc_error_code_from_response(&send_response, "eth_sendRawTransaction"),
        "estimate and send should reject with the same JSON-RPC error class"
    );
    assert!(
        rpc_error_message(rpc_error_object(&send_response, "eth_sendRawTransaction"))
            .contains(INSUFFICIENT_FUNDS_FOR_GAS_ERROR),
        "raw send should report insufficient-funds affordability failure: {send_response}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn rpc2_003_eth_call_default_gas_uses_tx_cap() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let sender = ws_client.address();
    let http = Client::new();

    let deploy_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendTransaction",
        json!([{
            "from": sender,
            "data": GAS_LEFT_CONTRACT_DEPLOY_CODE,
            "gas": "0x2dc6c0",
            "value": "0x0",
            "maxFeePerGas": hex_u128(DEFAULT_MAX_FEE_PER_GAS),
            "maxPriorityFeePerGas": hex_u128(DEFAULT_MAX_PRIORITY_FEE_PER_GAS)
        }]),
    )
    .await?;

    assert!(
        deploy_response.get("error").is_none(),
        "deployment should succeed: {deploy_response}"
    );

    let deploy_hash: B256 = rpc_result_hex(&deploy_response).parse()?;
    let deploy_receipt = ws_client.wait_for_receipt(deploy_hash).await;
    assert!(
        deploy_receipt.status(),
        "deployment should execute successfully"
    );
    let contract_address = deploy_receipt
        .contract_address
        .expect("deployment should return contract address");
    let deployed_code: Bytes = ws_client
        .ws
        .request("eth_getCode", rpc_params![contract_address, "latest"])
        .await?;
    assert!(
        deployed_code.len() >= GAS_LEFT_CONTRACT_RUNTIME_CODE.len(),
        "deployment should return runtime code, got {deployed_code:?}"
    );
    assert_eq!(
        &deployed_code[..GAS_LEFT_CONTRACT_RUNTIME_CODE.len()],
        GAS_LEFT_CONTRACT_RUNTIME_CODE,
        "deployment should publish the expected gasleft runtime"
    );

    let no_gas_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_call",
        json!([
            {
                "to": contract_address,
                "data": "0x"
            },
            "latest"
        ]),
    )
    .await?;

    let capped_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_call",
        json!([
            {
                "to": contract_address,
                "data": "0x",
                "gas": hex_u64(ETH_TX_GAS_CAP)
            },
            "latest"
        ]),
    )
    .await?;

    assert!(
        no_gas_response.get("error").is_none() && capped_response.get("error").is_none(),
        "eth_call variants should both succeed: no_gas={no_gas_response}, capped={capped_response}"
    );
    let no_gas_result = rpc_result_hex(&no_gas_response);
    let capped_result = rpc_result_hex(&capped_response);
    assert_ne!(
        no_gas_result, "0x",
        "omitted-gas eth_call should return ABI-encoded gasleft bytes"
    );
    assert_ne!(
        capped_result, "0x",
        "capped eth_call should return ABI-encoded gasleft bytes"
    );
    assert_eq!(
        no_gas_result.len(),
        66,
        "omitted-gas eth_call should return a 32-byte ABI word"
    );
    assert_eq!(
        capped_result.len(),
        66,
        "capped eth_call should return a 32-byte ABI word"
    );

    let gas_left_without_cap = parse_hex_u64(&no_gas_result);
    let gas_left_with_cap = parse_hex_u64(&capped_result);

    assert!(
        gas_left_without_cap <= gas_left_with_cap.saturating_add(500_000),
        "omitted gas should not execute with materially higher gas budget than tx gas cap (without_cap={gas_left_without_cap}, with_cap={gas_left_with_cap})"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn rpc2_004_estimate_gas_tracks_receipt_gas_used() -> anyhow::Result<()> {
    let (_rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let tx_request = client.make_tx(Some(contract), Some(client.contract.set(0x1337)));
    let estimate: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&tx_request, "latest"])
        .await?;

    let tx_hash = client.set_value(contract, 0x1337).await;
    let receipt = client.wait_for_receipt(tx_hash).await;

    let estimate_value = estimate.to::<u64>();
    assert!(
        estimate_value.abs_diff(receipt.gas_used) < 10_000,
        "estimate should be near actual execution gasUsed (estimate={estimate_value}, gasUsed={})",
        receipt.gas_used
    );

    Ok(())
}

// Overlap note: receipt fee reconciliation is also covered by
// `evm_rpc_compliance_validation.rs::rpc_008_receipt_fee_fields_match_balance_delta`
// and `sov-evm/tests/integration/transactions.rs::test_block_receipt_fee_matches_balance_delta`.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_005_receipt_fee_fields_reconcile_exactly_with_balance_delta() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

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
        "sender balance delta should exactly match value + receipt gas cost"
    );

    Ok(())
}

// Overlap note: zero-count fee history coverage already exists in
// `evm_fee_history.rs::test_eth_fee_history_zero_blocks`.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_006_fee_history_zero_block_count_returns_empty_response() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let http = Client::new();

    let response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_feeHistory",
        json!(["0x0", "latest", []]),
    )
    .await?;

    assert!(
        response.get("error").is_none(),
        "eth_feeHistory(0, ...) should return empty result, not error: {response}"
    );

    assert_eq!(response["result"]["oldestBlock"].as_str(), Some("0x0"));
    assert_eq!(
        response["result"]["baseFeePerGas"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        0
    );
    assert_eq!(
        response["result"]["gasUsedRatio"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        0
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "low priority now"]
async fn rpc2_007_fee_history_reward_percentiles_reflect_tipped_transactions() -> anyhow::Result<()>
{
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let http = Client::new();
    let mut tx = ws_client.make_tx(Some(Address::repeat_byte(0x55)), None);
    tx = tx
        .value(U256::from(1u64))
        .max_fee_per_gas(HIGH_MAX_FEE_PER_GAS)
        .max_priority_fee_per_gas(HIGH_PRIORITY_FEE_PER_GAS);

    let receipt = ws_client
        .send_tx_and_wait_finalized(tx)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let tx_hash = receipt.transaction_hash;
    let block_number = receipt
        .block_number
        .ok_or_else(|| anyhow::anyhow!("finalized receipt should include block_number"))?;

    let tx_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getTransactionByHash",
        json!([format!("{tx_hash:#x}")]),
    )
    .await?;
    assert!(
        tx_response.get("error").is_none(),
        "eth_getTransactionByHash should succeed for tipped transaction: {tx_response}"
    );

    let max_priority_fee_hex = tx_response["result"]["maxPriorityFeePerGas"]
        .as_str()
        .expect("EIP-1559 transaction should include maxPriorityFeePerGas");
    let max_priority_fee = parse_hex_u128(max_priority_fee_hex);
    assert!(
        max_priority_fee > 0,
        "covered transaction should carry a non-zero maxPriorityFeePerGas"
    );

    let fee_history = rpc_call(
        &http,
        rollup.http_addr,
        "eth_feeHistory",
        json!(["0x1", hex_u64(block_number), [50]]),
    )
    .await?;

    assert!(
        fee_history.get("error").is_none(),
        "feeHistory request should succeed: {fee_history}"
    );

    let reward_hex = fee_history["result"]["reward"][0][0]
        .as_str()
        .expect("reward percentile should be present");
    let reward_value = parse_hex_u128(reward_hex);
    assert!(
        reward_value > 0,
        "reward percentile should reflect non-zero tip payments"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known issue. To be discussed and prioritized"]
async fn rpc2_008_block_omits_withdrawals_fields() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let http = Client::new();
    let response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!(["latest", false]),
    )
    .await?;

    assert!(
        response.get("error").is_none(),
        "eth_getBlockByNumber should succeed: {response}"
    );

    let block = &response["result"];
    assert!(
        block.get("withdrawals").is_none(),
        "block should omit withdrawals when they are unavailable: {block}"
    );
    assert_eq!(
        block.get("withdrawalsRoot"),
        None,
        "block should omit withdrawalsRoot when it is unavailable: {block}"
    );

    Ok(())
}

// Overlap note: `evm_block_by_number_hash.rs::test_get_block_by_hash_nonexistent`
// already covers the core null-on-missing-hash behavior.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_009_hash_not_found_semantics_are_consistent_across_block_endpoints(
) -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let http = Client::new();

    let unknown_hash = format!("{:#x}", B256::repeat_byte(0x42));

    let by_hash = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByHash",
        json!([unknown_hash, false]),
    )
    .await?;
    let tx_count_by_hash = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockTransactionCountByHash",
        json!([unknown_hash]),
    )
    .await?;
    let receipts_by_hash = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockReceipts",
        json!([unknown_hash]),
    )
    .await?;

    for (name, response) in [
        ("eth_getBlockByHash", by_hash),
        ("eth_getBlockTransactionCountByHash", tx_count_by_hash),
        ("eth_getBlockReceipts", receipts_by_hash),
    ] {
        assert!(
            response.get("error").is_none(),
            "{name} should return null for unknown hash, not error: {response}"
        );
        assert!(
            response["result"].is_null(),
            "{name} should return result=null for unknown hash: {response}"
        );
    }

    Ok(())
}

// Overlap note: default tracer coverage also exists in
// `evm_tracing.rs::debug_trace_block_by_number_default_tracer` and
// `sov-evm/tests/integration/trace.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_010_default_debug_trace_transaction_is_supported() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    let tx_hash = client
        .send_eth(Address::repeat_byte(0x66), U256::from(1u64))
        .await;
    let _receipt = client.wait_for_receipt(tx_hash).await;

    let http = Client::new();
    let response = rpc_call(
        &http,
        rollup.http_addr,
        "debug_traceTransaction",
        json!([format!("{tx_hash:#x}")]),
    )
    .await?;

    assert!(
        response.get("error").is_none(),
        "debug_traceTransaction without explicit tracer should be supported: {response}"
    );
    assert!(
        response.get("result").is_some() && !response["result"].is_null(),
        "default trace response should be non-null"
    );
    let trace: GethTrace = serde_json::from_value(response["result"].clone())?;
    match trace {
        GethTrace::Default(_) => {}
        other => panic!("expected default tracer result, got {other:?}"),
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "We don't have pending txs. To be discussed"]
async fn rpc2_011_new_pending_transactions_subscription_is_supported() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    let sub_id: Result<String, _> = ws_client
        .ws
        .request("eth_subscribe", rpc_params!["newPendingTransactions"])
        .await;

    assert!(
        sub_id.is_ok(),
        "eth_subscribe(newPendingTransactions) should be supported"
    );

    if let Ok(id) = sub_id {
        let _: Result<bool, _> = ws_client
            .ws
            .request("eth_unsubscribe", rpc_params![id])
            .await;
    }

    Ok(())
}

// Overlap note: block-tag consistency is also covered by
// `evm_block_by_number_hash.rs::{test_block_tags_earliest_safe_finalized, test_block_number_consistency}`.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_012_safe_and_finalized_tags_match_latest_on_instant_finality_chain(
) -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(5).await;

    let http = Client::new();
    let latest = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!(["latest", false]),
    )
    .await?;
    let safe = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!(["safe", false]),
    )
    .await?;
    let finalized = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!(["finalized", false]),
    )
    .await?;

    assert!(latest.get("error").is_none());
    assert!(safe.get("error").is_none());
    assert!(finalized.get("error").is_none());

    let latest_number = parse_hex_u64(
        latest["result"]["number"]
            .as_str()
            .expect("latest.number should be set"),
    );
    let safe_number = parse_hex_u64(
        safe["result"]["number"]
            .as_str()
            .expect("safe.number should be set"),
    );
    let finalized_number = parse_hex_u64(
        finalized["result"]["number"]
            .as_str()
            .expect("finalized.number should be set"),
    );

    assert_eq!(
        safe_number, latest_number,
        "safe should match latest in instant-finality expectation"
    );
    assert_eq!(
        finalized_number, latest_number,
        "finalized should match latest in instant-finality expectation"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: synthetic hash lifecycle."]
// TODO: Why this is a problem?
async fn rpc2_013_synthetic_block_hash_remains_resolvable_after_sealing() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    rollup.pause_preferred_batches().await;
    let tx_hash = client
        .send_eth(Address::repeat_byte(0x88), U256::from(1u64))
        .await;
    let _receipt = client.wait_for_receipt(tx_hash).await;

    let http = Client::new();
    let pending_block = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!(["pending", false]),
    )
    .await?;

    assert!(
        pending_block.get("error").is_none(),
        "pending block query should succeed: {pending_block}"
    );
    let pending_hash = pending_block["result"]["hash"]
        .as_str()
        .expect("pending block hash should be present")
        .to_string();

    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(30).await;

    let by_hash = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByHash",
        json!([pending_hash, false]),
    )
    .await?;

    assert!(
        by_hash.get("error").is_none(),
        "synthetic hash should remain queryable after sealing/pruning: {by_hash}"
    );
    assert!(
        by_hash["result"].is_object(),
        "synthetic hash should resolve to a block object"
    );

    Ok(())
}

// Overlap note: missing-block null semantics are also covered by
// `evm_block_by_number_hash.rs::test_nonexistent_block_returns_none`.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_014_future_numeric_block_selector_returns_null() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let http = Client::new();
    let latest = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!(["latest", false]),
    )
    .await?;
    assert!(latest.get("error").is_none());

    let latest_number = parse_hex_u64(
        latest["result"]["number"]
            .as_str()
            .expect("latest.number should be present"),
    );
    let future_selector = hex_u64(latest_number.saturating_add(1));

    let future = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!([future_selector, false]),
    )
    .await?;

    assert!(
        future.get("error").is_none(),
        "future block query should return null, not error: {future}"
    );
    assert!(
        future["result"].is_null(),
        "future block query should return null"
    );

    Ok(())
}
