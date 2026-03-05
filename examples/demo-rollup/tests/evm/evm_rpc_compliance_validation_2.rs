use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256, U64};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use reqwest::Client;
use serde_json::json;

use crate::evm::evm_test_helper::{
    create_simple_storage_client, deploy_contract_check, hex_u128, hex_u64, parse_hex_u128,
    parse_hex_u64, raw_signed_eip1559, rpc_call, rpc_error_code_from_response, rpc_result_hex,
    setup_test_rollup, setup_with_simple_storage, tx_count, EVM_EXTENSION, SENDER_PRIV_KEY,
};

const DEFAULT_MAX_FEE_PER_GAS: u128 = 1_000_000_000;
const DEFAULT_MAX_PRIORITY_FEE_PER_GAS: u128 = 1;
const ETH_TX_GAS_CAP: u64 = 30_000_000;
const EMPTY_WITHDRAWALS_ROOT: &str =
    "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421";
const GASLEFT_CONTRACT_DEPLOY_CODE: &str = "0x6008600c60003960086000f35a60005260206000f3";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: estimate/send maxFee admission mismatch"]
async fn rpc2_001_estimate_send_max_fee_admission_consistency() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

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
#[ignore = "Known compatibility gap: estimate/send affordability mismatch"]
async fn rpc2_002_estimate_send_affordability_consistency() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let nonce = tx_count(&ws_client, signer.address(), "latest").await?;
    let chain_id: U64 = ws_client.ws.request("eth_chainId", rpc_params![]).await?;

    let huge_max_fee = u128::MAX / 4;
    let estimate_request = json!({
        "from": signer.address(),
        "to": Address::repeat_byte(0x22),
        "value": "0x0",
        "gas": "0x5208",
        "maxFeePerGas": hex_u128(huge_max_fee),
        "maxPriorityFeePerGas": "0x1"
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
        TxKind::Call(Address::repeat_byte(0x22)),
        U256::ZERO,
        Bytes::new(),
        huge_max_fee,
        1,
    )
    .await?;

    let send_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendRawTransaction",
        json!([raw_tx]),
    )
    .await?;

    let estimate_is_error = estimate_response.get("error").is_some();
    let send_is_error = send_response.get("error").is_some();

    assert_eq!(
        estimate_is_error, send_is_error,
        "estimate and send should agree on affordability classification: estimate={estimate_response}, send={send_response}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: eth_call omitted gas uses block gas context"]
async fn rpc2_003_eth_call_default_gas_uses_tx_cap() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let sender = ws_client.address();
    let http = Client::new();

    let deploy_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendTransaction",
        json!([{
            "from": sender,
            "data": GASLEFT_CONTRACT_DEPLOY_CODE,
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
    let contract_address = deploy_receipt
        .contract_address
        .expect("deployment should return contract address");

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

    let gas_left_without_cap = parse_hex_u64(
        no_gas_response["result"]
            .as_str()
            .expect("eth_call should return quantity bytes"),
    );
    let gas_left_with_cap = parse_hex_u64(
        capped_response["result"]
            .as_str()
            .expect("eth_call should return quantity bytes"),
    );

    assert!(
        gas_left_without_cap <= gas_left_with_cap.saturating_add(500_000),
        "omitted gas should not execute with materially higher gas budget than tx gas cap (without_cap={gas_left_without_cap}, with_cap={gas_left_with_cap})"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: eth_estimateGas returns sovereign gas units"]
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
        estimate_value <= receipt.gas_used.saturating_add(10_000),
        "estimate should be near actual execution gasUsed (estimate={estimate_value}, gasUsed={})",
        receipt.gas_used
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: exact receipt fee reconciliation"]
async fn rpc2_005_receipt_fee_fields_reconcile_exactly_with_balance_delta() -> anyhow::Result<()> {
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
        "sender balance delta should exactly match value + receipt gas cost"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: eth_feeHistory(block_count=0) behavior"]
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
#[ignore = "Known compatibility gap: eth_feeHistory reward percentiles always zero"]
async fn rpc2_007_fee_history_reward_percentiles_reflect_tipped_transactions() -> anyhow::Result<()>
{
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let nonce = tx_count(&ws_client, signer.address(), "latest").await?;
    let chain_id: U64 = ws_client.ws.request("eth_chainId", rpc_params![]).await?;

    let raw_tx = raw_signed_eip1559(
        &signer,
        chain_id.to::<u64>(),
        nonce,
        21_000,
        TxKind::Call(Address::repeat_byte(0x55)),
        U256::ZERO,
        Bytes::new(),
        DEFAULT_MAX_FEE_PER_GAS,
        10_000,
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
        "tipped transaction should be accepted: {send_response}"
    );

    let tx_hash: B256 = rpc_result_hex(&send_response).parse()?;
    let _receipt = ws_client.wait_for_finalized_receipt(tx_hash).await;

    let fee_history = rpc_call(
        &http,
        rollup.http_addr,
        "eth_feeHistory",
        json!(["0x1", "latest", [50]]),
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
#[ignore = "Known compatibility gap: withdrawals schema for post-Cancun blocks"]
async fn rpc2_008_post_cancun_block_reports_empty_withdrawals_array() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

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
        block["withdrawals"].is_array(),
        "post-Cancun block should return withdrawals as an empty array"
    );
    assert_eq!(
        block["withdrawals"].as_array().map(Vec::len),
        Some(0),
        "post-Cancun block should return empty withdrawals array"
    );
    assert_eq!(
        block["withdrawalsRoot"].as_str(),
        Some(EMPTY_WITHDRAWALS_ROOT),
        "post-Cancun block should include canonical empty withdrawalsRoot"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: hash not-found semantics are inconsistent"]
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: debug_traceTransaction default tracer unsupported"]
async fn rpc2_010_default_debug_trace_transaction_is_supported() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
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

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: newPendingTransactions subscription unsupported"]
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: safe/finalized lag latest"]
async fn rpc2_012_safe_and_finalized_tags_match_latest_on_instant_finality_chain(
) -> anyhow::Result<()> {
    let rollup = setup_test_rollup(2, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(5).await;

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
#[ignore = "Known compatibility gap: synthetic hash lifecycle"]
async fn rpc2_013_synthetic_block_hash_remains_resolvable_after_sealing() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

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
    rollup.wait_for_next_blocks(30).await;

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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: explicit future block selector behavior"]
async fn rpc2_014_future_numeric_block_selector_returns_null() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

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
