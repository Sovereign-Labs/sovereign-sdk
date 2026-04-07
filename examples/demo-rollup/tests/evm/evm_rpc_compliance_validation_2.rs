use crate::evm::evm_test_helper::{
    alloy_ws_client, call_all_endpoints, create_simple_storage_client, deploy_contract_check,
    hex_u128, hex_u64, parse_hex_u128, parse_hex_u64, raw_signed_eip1559, rpc_call,
    rpc_error_code_from_response, rpc_error_message, rpc_error_object, rpc_result_hex,
    setup_test_rollup, setup_test_rollup_with_paymaster, setup_with_simple_storage, tx_count,
    EVM_EXTENSION, FEE_CAP_TOO_LOW_ERROR, HIGH_MAX_FEE_PER_GAS, HIGH_PRIORITY_FEE_PER_GAS,
    INSUFFICIENT_FUNDS_ERROR, MAX_FEE_PER_GAS, PAYER_SOV_BANK_BALANCE, SENDER_PRIV_KEY,
};
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256, U64};
use alloy_provider::Provider;
use alloy_rpc_types_eth::{Filter, TransactionRequest};
use alloy_rpc_types_trace::geth::GethTrace;
use futures::StreamExt;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use reqwest::Client;
use serde_json::json;
use tokio::time::{timeout, Duration};

const DEFAULT_MAX_PRIORITY_FEE_PER_GAS: u128 = 1;
const ETH_TX_GAS_CAP: u64 = 30_000_000;
const GAS_LEFT_CONTRACT_DEPLOY_CODE: &str = "0x6009600c60003960096000f35a60005260206000f3";
const GAS_LEFT_CONTRACT_RUNTIME_CODE: &[u8] =
    &[0x5a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3];
const AFFORDABILITY_SIGNER_PRIV_KEY: &str =
    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const EMPTY_WITHDRAWALS_ROOT: &str =
    "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421";
const SUBSCRIPTION_TIMEOUT: Duration = Duration::from_secs(10);

fn assert_nonce_error_precedes_affordability(response: &serde_json::Value, method: &str) {
    let error_message = rpc_error_message(rpc_error_object(response, method));
    assert!(
        error_message.contains("nonce too low")
            || error_message.contains("Tx bad nonce")
            || error_message.contains("nonce"),
        "{method} should surface a nonce failure before affordability: {response}"
    );
    assert!(
        !error_message.contains(INSUFFICIENT_FUNDS_ERROR),
        "{method} should not surface insufficient funds when the nonce is already invalid: {response}"
    );
}

/// RPC2-002c: Preferred sequencer raw send should accept a future nonce when the gap is filled before timeout.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_002c_preferred_raw_send_accepts_future_nonce() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&ws_client)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let chain_id: U64 = ws_client.ws.request("eth_chainId", rpc_params![]).await?;
    let current_nonce = tx_count(&ws_client, signer.address(), "latest").await?;
    let mut estimate_request =
        ws_client.make_tx(Some(contract_address), Some(ws_client.contract.set(1)));
    estimate_request.gas = None;
    let gas_limit: U64 = ws_client
        .ws
        .request("eth_estimateGas", rpc_params![&estimate_request, "latest"])
        .await?;
    let gas_limit = gas_limit.to::<u64>();
    let max_fee_per_gas = 100u128;
    let max_priority_fee_per_gas = DEFAULT_MAX_PRIORITY_FEE_PER_GAS;
    let future_value = 22u32;
    let current_value = 11u32;
    let http = Client::new();

    let future_raw_tx = raw_signed_eip1559(
        &signer,
        chain_id.to::<u64>(),
        current_nonce + 1,
        gas_limit,
        TxKind::Call(contract_address),
        U256::ZERO,
        ws_client.contract.set(future_value),
        max_fee_per_gas,
        max_priority_fee_per_gas,
    )
    .await?;
    let future_http = http.clone();
    let future_addr = rollup.http_addr;
    let future_handle = tokio::spawn(async move {
        rpc_call(
            &future_http,
            future_addr,
            "eth_sendRawTransaction",
            json!([future_raw_tx]),
        )
        .await
    });

    // Preferred-sequencer sends are synchronous in this harness, so the future-nonce
    // request must already be waiting in the reorder queue before we submit the gap-filling tx.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let current_raw_tx = raw_signed_eip1559(
        &signer,
        chain_id.to::<u64>(),
        current_nonce,
        gas_limit,
        TxKind::Call(contract_address),
        U256::ZERO,
        ws_client.contract.set(current_value),
        max_fee_per_gas,
        max_priority_fee_per_gas,
    )
    .await?;
    let current_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendRawTransaction",
        json!([current_raw_tx]),
    )
    .await?;
    assert!(
        current_response.get("error").is_none(),
        "current-nonce raw tx should succeed while draining the future-nonce queue: {current_response}"
    );
    let current_hash: B256 = rpc_result_hex(&current_response).parse()?;

    let future_response = future_handle.await??;
    assert!(
        future_response.get("error").is_none(),
        "preferred sequencer should complete the queued future raw tx once the missing nonce arrives: {future_response}"
    );
    let future_hash: B256 = rpc_result_hex(&future_response).parse()?;

    let current_receipt = ws_client.wait_for_finalized_receipt(current_hash).await;
    assert!(
        current_receipt.status(),
        "current-nonce raw tx should execute successfully"
    );

    let future_receipt = ws_client.wait_for_finalized_receipt(future_hash).await;
    assert!(
        future_receipt.status(),
        "queued future-nonce raw tx should execute after the missing nonce arrives"
    );

    assert_eq!(
        tx_count(&ws_client, signer.address(), "latest").await?,
        current_nonce + 2,
        "both raw txs should advance the account nonce once executed"
    );
    assert_eq!(
        ws_client.eth_get_storage_at(contract_address, U256::ZERO).await,
        U256::from(future_value),
        "future-nonce raw tx should execute after the current nonce and leave the last written value"
    );

    Ok(())
}

/// RPC2-002d: Preferred sequencer local send should accept a future nonce when the gap is filled before timeout.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_002d_preferred_local_send_accepts_future_nonce() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&ws_client)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let current_nonce = tx_count(&ws_client, ws_client.address(), "latest").await?;
    let mut estimate_request =
        ws_client.make_tx(Some(contract_address), Some(ws_client.contract.set(1)));
    estimate_request.gas = None;
    let gas_limit: U64 = ws_client
        .ws
        .request("eth_estimateGas", rpc_params![&estimate_request, "latest"])
        .await?;
    let gas_limit = gas_limit.to::<u64>();
    let max_fee_per_gas = 100u128;
    let max_priority_fee_per_gas = DEFAULT_MAX_PRIORITY_FEE_PER_GAS;
    let future_value = 44u32;
    let current_value = 33u32;
    let http = Client::new();

    let future_http = http.clone();
    let future_addr = rollup.http_addr;
    let sender = ws_client.address();
    let future_data = format!(
        "0x{}",
        hex::encode(ws_client.contract.set(future_value).as_ref())
    );
    let future_handle = tokio::spawn(async move {
        rpc_call(
            &future_http,
            future_addr,
            "eth_sendTransaction",
            json!([{
                "from": sender,
                "to": contract_address,
                "data": future_data,
                "gas": hex_u64(gas_limit),
                "nonce": hex_u64(current_nonce + 1),
                "value": "0x0",
                "maxFeePerGas": hex_u128(max_fee_per_gas),
                "maxPriorityFeePerGas": hex_u128(max_priority_fee_per_gas)
            }]),
        )
        .await
    });

    // The future-nonce request must be inflight before the predecessor is sent,
    // otherwise the synchronous RPC path can complete nonce 0 first and skip queue coverage.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let current_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendTransaction",
        json!([{
            "from": ws_client.address(),
            "to": contract_address,
            "data": format!("0x{}", hex::encode(ws_client.contract.set(current_value).as_ref())),
            "gas": hex_u64(gas_limit),
            "nonce": hex_u64(current_nonce),
            "value": "0x0",
            "maxFeePerGas": hex_u128(max_fee_per_gas),
            "maxPriorityFeePerGas": hex_u128(max_priority_fee_per_gas)
        }]),
    )
    .await?;
    assert!(
        current_response.get("error").is_none(),
        "current-nonce local tx should succeed while draining the future-nonce queue: {current_response}"
    );
    let current_hash: B256 = rpc_result_hex(&current_response).parse()?;

    let future_response = future_handle.await??;
    assert!(
        future_response.get("error").is_none(),
        "preferred sequencer should complete the queued future local tx once the missing nonce arrives: {future_response}"
    );
    let future_hash: B256 = rpc_result_hex(&future_response).parse()?;

    let current_receipt = ws_client.wait_for_finalized_receipt(current_hash).await;
    assert!(
        current_receipt.status(),
        "current-nonce local tx should execute successfully"
    );

    let future_receipt = ws_client.wait_for_finalized_receipt(future_hash).await;
    assert!(
        future_receipt.status(),
        "queued future-nonce local tx should execute after the missing nonce arrives"
    );

    assert_eq!(
        tx_count(&ws_client, ws_client.address(), "latest").await?,
        current_nonce + 2,
        "both local txs should advance the account nonce once executed"
    );
    assert_eq!(
        ws_client.eth_get_storage_at(contract_address, U256::ZERO).await,
        U256::from(future_value),
        "future-nonce local tx should execute after the current nonce and leave the last written value"
    );

    Ok(())
}

/// RPC2-001: Fee-cap admission consistency across simulation and submission.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_001_estimate_send_max_fee_admission_consistency() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let nonce = tx_count(&ws_client, signer.address(), "latest").await?;
    let base_fee: U256 = ws_client.ws.request("eth_gasPrice", rpc_params![]).await?;
    let base_fee_u128: u128 = base_fee.to::<u128>();
    assert!(
        base_fee_u128 > 0,
        "base fee should be non-zero for this test"
    );

    let request = TransactionRequest {
        from: Some(signer.address()),
        to: Some(TxKind::Call(Address::repeat_byte(0x11))),
        value: Some(U256::ZERO),
        gas: Some(21_000),
        max_fee_per_gas: Some(0),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    };

    let results = call_all_endpoints(&ws_client, &request, &signer).await;

    // estimateGas, call, and sendRawTransaction must all reject
    let estimate_err = results.estimate_gas.unwrap_err();
    let call_err = results.call.unwrap_err();
    let send_err = results.send_raw_tx.unwrap_err();

    assert!(
        estimate_err.to_string().contains(FEE_CAP_TOO_LOW_ERROR),
        "estimate should reject for below-base-fee reason: {estimate_err}"
    );
    assert!(
        call_err.to_string().contains(FEE_CAP_TOO_LOW_ERROR),
        "eth_call should reject for below-base-fee reason: {call_err}"
    );
    assert!(
        send_err.to_string().contains(FEE_CAP_TOO_LOW_ERROR),
        "send should reject for below-base-fee reason: {send_err}"
    );

    // createAccessList is known to accept below-base-fee requests (it does not
    // check the fee cap). This is a documented divergence from the other endpoints.
    assert!(
        results.create_access_list.is_ok(),
        "createAccessList accepts low base fee (known difference from other endpoints): {:?}",
        results.create_access_list,
    );

    assert_eq!(
        tx_count(&ws_client, signer.address(), "latest").await?,
        nonce,
        "failed raw send must not advance sender nonce"
    );

    Ok(())
}

/// RPC2-001b: Mixed stale-nonce + low-fee input should reject consistently
/// across simulation and submission.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_001b_estimate_call_send_mixed_nonce_fee_mismatch() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;

    // Consume nonce 0 so an explicit nonce=0 becomes stale for all subsequent requests.
    let first_tx = ws_client
        .send_eth(Address::repeat_byte(0x11), U256::from(1u64))
        .await;
    ws_client.wait_for_receipt(first_tx).await;

    let next_nonce = tx_count(&ws_client, signer.address(), "latest").await?;
    assert!(next_nonce > 0, "nonce should advance after the first tx");

    let base_fee: U256 = ws_client.ws.request("eth_gasPrice", rpc_params![]).await?;
    let base_fee_u128 = base_fee.to::<u128>();
    assert!(
        base_fee_u128 > 0,
        "base fee should be non-zero for this test"
    );

    let low_fee = base_fee_u128 - 1;
    let request = TransactionRequest {
        from: Some(signer.address()),
        to: Some(TxKind::Call(Address::repeat_byte(0x22))),
        value: Some(U256::ZERO),
        gas: Some(21_000),
        nonce: Some(0),
        max_fee_per_gas: Some(low_fee),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    };

    let results = call_all_endpoints(&ws_client, &request, &signer).await;

    // estimateGas, call, and sendRawTransaction must all reject with fee-cap-too-low
    let estimate_err = results.estimate_gas.unwrap_err();
    let call_err = results.call.unwrap_err();
    let send_err = results.send_raw_tx.unwrap_err();

    assert!(
        estimate_err.to_string().contains(FEE_CAP_TOO_LOW_ERROR),
        "estimate should prefer fee-cap-too-low in the mixed case: {estimate_err}"
    );
    assert!(
        call_err.to_string().contains(FEE_CAP_TOO_LOW_ERROR),
        "eth_call should prefer fee-cap-too-low in the mixed case: {call_err}"
    );
    assert!(
        send_err.to_string().contains(FEE_CAP_TOO_LOW_ERROR),
        "raw send should reject for fee-cap-too-low first: {send_err}"
    );

    // createAccessList is known to accept below-base-fee requests even with a stale
    // nonce (it checks neither fee cap nor nonce). Documented divergence.
    assert!(
        results.create_access_list.is_ok(),
        "createAccessList accepts low fee + stale nonce (known difference): {:?}",
        results.create_access_list,
    );

    assert_eq!(
        tx_count(&ws_client, signer.address(), "latest").await?,
        next_nonce,
        "failed raw send must not advance sender nonce"
    );

    Ok(())
}

/// RPC2-002: Affordability check consistency between estimation and send path.
///
/// Funds the account with a tiny balance (less than `gas_limit * rollup_gas_price`)
/// so that both estimate and send reject with insufficient-funds at the actual
/// gas cost threshold (not the user-declared `maxFeePerGas`).
#[tokio::test(flavor = "multi_thread")]
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
    let gas_cost_floor = U256::from(gas_limit) * U256::from(initial_rollup_gas_price);
    // Fund with a tiny amount: less than the actual gas cost at rollup base fee.
    let affordability_balance = U256::from(1000u64);

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
        affordability_balance < gas_cost_floor,
        "funding balance ({affordability_balance}) must be below gas cost floor ({gas_cost_floor})"
    );

    let funding_hash = ws_client
        .send_eth(affordability_address, affordability_balance)
        .await;
    let funding_receipt = ws_client.wait_for_receipt(funding_hash).await;
    assert!(funding_receipt.status(), "funding transfer should succeed");
    assert_eq!(
        ws_client.eth_get_balance(affordability_address).await,
        affordability_balance,
        "recipient balance should exactly match the funded amount"
    );

    let nonce = tx_count(&ws_client, affordability_address, "latest").await?;
    assert_eq!(nonce, 0, "receiving funds should not change sender nonce");
    let estimate_request = json!({
        "from": affordability_address,
        "to": recipient,
        "value": "0x0",
        "maxFeePerGas": hex_u128(MAX_FEE_PER_GAS),
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
        MAX_FEE_PER_GAS,
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
    let estimate_msg = rpc_error_message(rpc_error_object(&estimate_response, "eth_estimateGas"));
    assert!(
        // The EVM-level check returns INSUFFICIENT_FUNDS_ERROR; the paymaster-aware
        // try_reserve_gas path returns "Insufficient balance …". Both are valid.
        estimate_msg.contains(INSUFFICIENT_FUNDS_ERROR)
            || estimate_msg.contains("Insufficient balance"),
        "estimate should report insufficient-funds affordability failure: {estimate_response}"
    );

    assert_eq!(
        rpc_error_code_from_response(&estimate_response, "eth_estimateGas"),
        rpc_error_code_from_response(&send_response, "eth_sendRawTransaction"),
        "estimate and send should reject with the same JSON-RPC error class"
    );
    assert!(
        rpc_error_message(rpc_error_object(&send_response, "eth_sendRawTransaction"))
            .contains(INSUFFICIENT_FUNDS_ERROR),
        "raw send should report insufficient-funds affordability failure: {send_response}"
    );

    Ok(())
}

/// RPC2-002b: Stale nonce must win over affordability preflights on estimate and send paths.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_002b_stale_nonce_precedes_affordability_rejection() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let sender = signer.address();
    let chain_id: U64 = ws_client.ws.request("eth_chainId", rpc_params![]).await?;
    let recipient = Address::repeat_byte(0x24);
    let gas_limit = 21_000u64;

    let first_tx = ws_client.send_eth(recipient, U256::from(1u64)).await;
    let first_receipt = ws_client.wait_for_receipt(first_tx).await;
    assert!(
        first_receipt.status(),
        "first tx should advance the sender nonce"
    );

    let next_nonce = tx_count(&ws_client, sender, "latest").await?;
    assert!(
        next_nonce > 0,
        "sender nonce should advance after the first tx"
    );
    let stale_nonce = next_nonce - 1;
    let sender_balance = ws_client.eth_get_balance(sender).await;
    let underfunded_value = sender_balance
        .checked_add(U256::from(1u64))
        .expect("test balance should allow adding one wei");

    let http = Client::new();
    let estimate_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        json!([{
            "from": sender,
            "to": recipient,
            "gas": hex_u64(gas_limit),
            "nonce": hex_u64(stale_nonce),
            "value": format!("{underfunded_value:#x}"),
            "maxFeePerGas": hex_u128(MAX_FEE_PER_GAS),
            "maxPriorityFeePerGas": "0x0"
        }, "latest"]),
    )
    .await?;
    assert_nonce_error_precedes_affordability(&estimate_response, "eth_estimateGas");

    let raw_tx = raw_signed_eip1559(
        &signer,
        chain_id.to::<u64>(),
        stale_nonce,
        gas_limit,
        TxKind::Call(recipient),
        underfunded_value,
        Bytes::new(),
        MAX_FEE_PER_GAS,
        0,
    )
    .await?;
    let raw_send_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendRawTransaction",
        json!([raw_tx]),
    )
    .await?;
    assert_eq!(
        rpc_error_code_from_response(&raw_send_response, "eth_sendRawTransaction"),
        -32602,
        "raw send should keep the invalid-params class for nonce failures"
    );
    assert_nonce_error_precedes_affordability(&raw_send_response, "eth_sendRawTransaction");

    let local_send_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendTransaction",
        json!([{
            "from": sender,
            "to": recipient,
            "gas": hex_u64(gas_limit),
            "nonce": hex_u64(stale_nonce),
            "value": format!("{underfunded_value:#x}"),
            "maxFeePerGas": hex_u128(MAX_FEE_PER_GAS),
            "maxPriorityFeePerGas": "0x0"
        }]),
    )
    .await?;
    assert_eq!(
        rpc_error_code_from_response(&local_send_response, "eth_sendTransaction"),
        -32602,
        "local send should keep the invalid-params class for nonce failures"
    );
    assert_nonce_error_precedes_affordability(&local_send_response, "eth_sendTransaction");

    assert_eq!(
        tx_count(&ws_client, sender, "latest").await?,
        next_nonce,
        "failed stale sends must not advance the sender nonce"
    );

    Ok(())
}

/// RPC2-002d: Explicit-gas estimate must preserve affordability precedence over
/// simulation gas failures so it matches the send path.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_002d_explicit_gas_affordability_precedes_simulation_failure() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = AFFORDABILITY_SIGNER_PRIV_KEY.parse()?;
    let sender = signer.address();

    assert_eq!(
        ws_client.eth_get_balance(sender).await,
        U256::ZERO,
        "fresh affordability signer must start unfunded"
    );
    assert!(
        ws_client.eth_gas_price().await > 0,
        "rollup gas price should be non-zero for this test"
    );

    let request = TransactionRequest {
        from: Some(sender),
        to: Some(TxKind::Call(Address::repeat_byte(0x25))),
        gas: Some(20_000),
        value: Some(U256::ZERO),
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    };

    let results = call_all_endpoints(&ws_client, &request, &signer).await;

    let estimate_err = results
        .estimate_gas
        .expect_err("estimate should reject the unfunded explicit-gas request")
        .to_string();
    let send_err = results
        .send_raw_tx
        .expect_err("raw send should reject the unfunded explicit-gas request")
        .to_string();

    assert!(
        estimate_err.contains(INSUFFICIENT_FUNDS_ERROR),
        "estimate should keep affordability precedence for explicit-gas requests: {estimate_err}"
    );
    assert!(
        send_err.contains(INSUFFICIENT_FUNDS_ERROR),
        "raw send should reject on affordability before execution: {send_err}"
    );
    assert!(
        !estimate_err.contains("out of gas") && !estimate_err.contains("intrinsic gas"),
        "estimate should not fall through to simulation gas failure first: {estimate_err}"
    );

    Ok(())
}

/// RPC2-002e: Using the estimate value as the explicit gas cap should result
/// in both a successful estimate and a successful send.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_002e_explicit_gas_equal_to_estimate_matches_send_success() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let recipient = Address::repeat_byte(0x26);

    // First, estimate without explicit gas to learn the projected gas value.
    let estimate_request = TransactionRequest {
        from: Some(signer.address()),
        to: Some(TxKind::Call(recipient)),
        value: Some(U256::ZERO),
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    };
    let estimated_gas: U64 = ws_client
        .ws
        .request("eth_estimateGas", rpc_params![&estimate_request, "latest"])
        .await
        .expect("estimate without explicit gas should succeed");
    let estimated_gas = estimated_gas.to::<u64>();

    // Now use that estimate as the explicit gas cap — both estimate and send
    // should succeed.
    let request = TransactionRequest {
        from: Some(signer.address()),
        to: Some(TxKind::Call(recipient)),
        gas: Some(estimated_gas),
        value: Some(U256::ZERO),
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    };

    let results = call_all_endpoints(&ws_client, &request, &signer).await;

    results
        .estimate_gas
        .expect("estimate should succeed when explicit gas equals the estimated value");

    let tx_hash = results
        .send_raw_tx
        .expect("raw send should succeed with the estimated gas limit");
    let receipt = ws_client.wait_for_receipt(tx_hash).await;
    assert!(
        receipt.status(),
        "transaction should execute successfully with the estimated gas limit"
    );

    Ok(())
}

/// RPC2-003 (gasleft probe): Omitted-gas `eth_call` should default to the tx gas cap.
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
            "maxFeePerGas": hex_u128(MAX_FEE_PER_GAS),
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

/// RPC2-003 (end-to-end burnGas): Omitted-gas simulation agrees with real tx cap on workload classification.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_003_omitted_gas_simulation_matches_real_tx_cap() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let chain_id: U64 = ws_client.ws.request("eth_chainId", rpc_params![]).await?;
    let contract_address = deploy_contract_check(&ws_client)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let http = Client::new();
    let sender = ws_client.address();
    let simulation_max_fee_per_gas = MAX_FEE_PER_GAS;
    let simulation_priority_fee_per_gas = DEFAULT_MAX_PRIORITY_FEE_PER_GAS;

    let capped_eth_call_succeeds = |calldata: Bytes| {
        let http = &http;
        async move {
            let response = rpc_call(
                http,
                rollup.http_addr,
                "eth_call",
                json!([{
                    "from": sender,
                    "to": contract_address,
                    "data": format!("0x{}", hex::encode(calldata.as_ref())),
                    "gas": hex_u64(ETH_TX_GAS_CAP),
                    "value": "0x0",
                    "maxFeePerGas": hex_u128(simulation_max_fee_per_gas),
                    "maxPriorityFeePerGas": hex_u128(simulation_priority_fee_per_gas)
                }, "latest"]),
            )
            .await?;
            Ok::<bool, anyhow::Error>(response.get("error").is_none())
        }
    };

    let mut last_success = None;
    let mut first_failure = None;
    let mut probe = 1u32;
    while probe <= 1_000_000 {
        if capped_eth_call_succeeds(ws_client.contract.burn_gas(probe)).await? {
            last_success = Some(probe);
            probe = probe
                .checked_mul(2)
                .ok_or_else(|| anyhow::anyhow!("burn_gas probe overflow"))?;
        } else {
            first_failure = Some(probe);
            break;
        }
    }
    let control_iterations = last_success
        .ok_or_else(|| anyhow::anyhow!("failed to find a workload below tx gas cap"))?;
    let failing_iterations = first_failure
        .ok_or_else(|| anyhow::anyhow!("failed to find a burn_gas workload beyond tx gas cap"))?;

    assert!(
        capped_eth_call_succeeds(ws_client.contract.burn_gas(control_iterations)).await?,
        "the largest observed successful workload should still fit inside the real tx gas cap"
    );

    let failing_calldata = ws_client.contract.burn_gas(failing_iterations);
    let explicit_capped_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_call",
        json!([{
            "from": sender,
            "to": contract_address,
            "data": format!("0x{}", hex::encode(failing_calldata.as_ref())),
            "gas": hex_u64(ETH_TX_GAS_CAP),
            "value": "0x0",
            "maxFeePerGas": hex_u128(simulation_max_fee_per_gas),
            "maxPriorityFeePerGas": hex_u128(simulation_priority_fee_per_gas)
        }, "latest"]),
    )
    .await?;
    assert!(
        explicit_capped_response.get("error").is_some(),
        "explicit 30M-gas eth_call should fail for the threshold workload: {explicit_capped_response}"
    );

    let omitted_gas_call_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_call",
        json!([{
            "from": sender,
            "to": contract_address,
            "data": format!("0x{}", hex::encode(failing_calldata.as_ref())),
            "value": "0x0",
            "maxFeePerGas": hex_u128(simulation_max_fee_per_gas),
            "maxPriorityFeePerGas": hex_u128(simulation_priority_fee_per_gas)
        }, "latest"]),
    )
    .await?;
    assert!(
        omitted_gas_call_response.get("error").is_some(),
        "omitted-gas eth_call should classify the threshold workload as failing too: {omitted_gas_call_response}"
    );

    let estimate_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        json!([{
            "from": sender,
            "to": contract_address,
            "data": format!("0x{}", hex::encode(failing_calldata.as_ref())),
            "value": "0x0",
            "maxFeePerGas": hex_u128(simulation_max_fee_per_gas),
            "maxPriorityFeePerGas": hex_u128(simulation_priority_fee_per_gas)
        }, "latest"]),
    )
    .await?;
    assert!(
        estimate_response.get("error").is_some(),
        "omitted-gas estimateGas should reject the threshold workload: {estimate_response}"
    );
    assert!(
        estimate_response.get("result").is_none() || estimate_response["result"].is_null(),
        "estimateGas should not return a gas value for the threshold workload: {estimate_response}"
    );

    let nonce = tx_count(&ws_client, signer.address(), "latest").await?;
    let raw_tx = raw_signed_eip1559(
        &signer,
        chain_id.to::<u64>(),
        nonce,
        ETH_TX_GAS_CAP,
        TxKind::Call(contract_address),
        U256::ZERO,
        failing_calldata,
        simulation_max_fee_per_gas,
        simulation_priority_fee_per_gas,
    )
    .await?;
    let send_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendRawTransaction",
        json!([raw_tx]),
    )
    .await?;
    if send_response.get("error").is_none() {
        let tx_hash: B256 = rpc_result_hex(&send_response).parse()?;
        let receipt = ws_client.wait_for_finalized_receipt(tx_hash).await;
        assert!(
            !receipt.status(),
            "a real 30M-gas tx should not complete successfully for the threshold workload"
        );
    }

    Ok(())
}

/// RPC2-004: `eth_estimateGas` units track receipt `gasUsed` (internal consistency).
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

/// RPC2-004b: `eth_estimateGas` tracks receipt `gasUsed` for a plain ETH transfer.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_004b_estimate_gas_tracks_receipt_for_eth_transfer() -> anyhow::Result<()> {
    let (_rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;

    let recipient = Address::repeat_byte(0x77);
    let value = U256::from(1_000_000u64);

    let tx_request = client.make_tx(Some(recipient), None).value(value);
    let estimate: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&tx_request, "latest"])
        .await?;

    let tx_hash = client.send_eth(recipient, value).await;
    let receipt = client.wait_for_receipt(tx_hash).await;

    let estimate_value = estimate.to::<u64>();
    assert!(
        estimate_value.abs_diff(receipt.gas_used) < 10_000,
        "ETH transfer: estimate should be near actual gasUsed (estimate={estimate_value}, gasUsed={})",
        receipt.gas_used
    );

    Ok(())
}

/// RPC2-004c: `eth_estimateGas` tracks receipt `gasUsed` for a contract deployment.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_004c_estimate_gas_tracks_receipt_for_contract_deploy() -> anyhow::Result<()> {
    let (_rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;

    let tx_request = client.make_tx(None, Some(client.contract.byte_code()));
    let estimate: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&tx_request, "latest"])
        .await?;

    let tx_hash = client
        .deploy_contract()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let receipt = client.wait_for_receipt(tx_hash).await;

    let estimate_value = estimate.to::<u64>();
    assert!(
        estimate_value.abs_diff(receipt.gas_used) < 10_000,
        "contract deploy: estimate should be near actual gasUsed (estimate={estimate_value}, gasUsed={})",
        receipt.gas_used
    );

    Ok(())
}

/// RPC2-004d: `eth_estimateGas` tracks receipt `gasUsed` for a log-emitting call.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_004d_estimate_gas_tracks_receipt_for_log_emission() -> anyhow::Result<()> {
    let (_rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let tx_request = client.make_tx(Some(contract), Some(client.contract.emit_logs(0x42, 3)));
    let estimate: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&tx_request, "latest"])
        .await?;

    let tx_hash = client.alloy_emit_logs(contract, 0x42, 3).await;
    let receipt = client.wait_for_receipt(tx_hash).await;

    let estimate_value = estimate.to::<u64>();
    assert!(
        estimate_value.abs_diff(receipt.gas_used) < 10_000,
        "log emission: estimate should be near actual gasUsed (estimate={estimate_value}, gasUsed={})",
        receipt.gas_used
    );

    Ok(())
}

/// RPC2-005: Receipt fee fields reconcile exactly with sender balance delta.
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

/// RPC2-006: `eth_feeHistory(block_count=0)` should return an empty result, not an error.
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
    assert!(
        response["result"]["baseFeePerGas"].is_null()
            || response["result"]["baseFeePerGas"] == json!([]),
        "baseFeePerGas should be empty or omitted for zero-block feeHistory"
    );
    assert_eq!(
        response["result"]["gasUsedRatio"],
        json!([]),
        "gasUsedRatio should be exactly [] for zero-block feeHistory"
    );
    assert!(
        response["result"].get("reward").is_none() || response["result"]["reward"].is_null(),
        "reward should be omitted for zero-block feeHistory with empty percentiles: {response}"
    );

    Ok(())
}

/// RPC2-007: `eth_feeHistory.reward` percentiles should reflect actual tips.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known discrepancy: low priority"]
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

/// RPC2-008: Post-Cancun blocks should include `withdrawals: []` and canonical empty `withdrawalsRoot`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known discrepancy: low priority"]
async fn rpc2_008_post_cancun_block_reports_empty_withdrawals_array() -> anyhow::Result<()> {
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
    let withdrawals = block
        .get("withdrawals")
        .and_then(|value| value.as_array())
        .expect("post-Cancun blocks should expose withdrawals as an array");
    assert_eq!(
        withdrawals.len(),
        0,
        "post-Cancun blocks should expose an empty withdrawals array: {block}"
    );
    assert_eq!(
        block
            .get("withdrawalsRoot")
            .and_then(|value| value.as_str()),
        Some(EMPTY_WITHDRAWALS_ROOT),
        "post-Cancun blocks should expose the canonical empty withdrawals root: {block}"
    );

    Ok(())
}

/// RPC2-009: Missing-hash behavior should be consistent across block endpoints.
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

/// RPC2-010: `debug_traceTransaction` should support the default tracer.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known discrepancy: will be fixed in the follow up"]
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

/// RPC2-011: `eth_subscribe("newPendingTransactions")` should be supported.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: newPendingTransactions subscription unsupported"]
async fn rpc2_011_new_pending_transactions_subscription_emits_pending_hashes() -> anyhow::Result<()>
{
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let provider = alloy_ws_client(rollup.http_addr).await;

    rollup.pause_preferred_batches_and_wait().await?;
    let sub = provider.subscribe_pending_transactions().await;
    assert!(
        sub.is_ok(),
        "eth_subscribe(newPendingTransactions) should be supported"
    );
    let mut stream = sub.expect("subscription should be created").into_stream();
    let tx_hash = ws_client
        .send_eth(Address::repeat_byte(0x99), U256::from(1u64))
        .await;
    let pending_hash = timeout(SUBSCRIPTION_TIMEOUT, stream.next())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for pending tx subscription event"))?
        .ok_or_else(|| anyhow::anyhow!("pending tx subscription closed without an event"))?;
    assert_eq!(
        pending_hash, tx_hash,
        "newPendingTransactions should emit the submitted pending tx hash"
    );

    Ok(())
}

/// RPC2-012: `safe` and `finalized` tags should match `latest` on an instant-finality chain.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_012_safe_and_finalized_tags_match_latest_on_instant_finality_chain(
) -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(5).await;
    // Freeze the head so all block-tag queries observe the same chain state.
    rollup.pause_preferred_batches_and_wait().await?;

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

/// RPC2-013: Synthetic pending `blockHash` should remain stable across the pending-to-sealed-to-pruned lifecycle.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: surfaced pending block hashes are not lifecycle-stable"]
async fn rpc2_013_surfaced_pending_block_hash_stays_stable_across_lifecycle() -> anyhow::Result<()>
{
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let ws_provider = alloy_ws_client(rollup.http_addr).await;
    let contract_address = client.alloy_deploy_contract().await;
    let http = Client::new();

    rollup.pause_preferred_batches_and_wait().await?;

    // Once a blockHash is surfaced to clients, it should remain the stable
    // cross-RPC identifier for that block across pending growth, sealing, and pruning.
    let mut log_stream = ws_provider
        .subscribe_logs(&Filter::new().address(contract_address))
        .await?
        .into_stream();

    let tx1 = client.alloy_emit_logs(contract_address, 1, 1).await;
    let first_log = timeout(SUBSCRIPTION_TIMEOUT, log_stream.next())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for first pending log"))?
        .ok_or_else(|| anyhow::anyhow!("log subscription closed before first event"))?;
    let first_hash = first_log
        .block_hash
        .expect("first pending log should include blockHash");
    let first_hash_hex = format!("{first_hash:#x}");
    let first_block_number = first_log
        .block_number
        .expect("first pending log should include blockNumber");

    let pending_tx1_before_growth = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getTransactionByHash",
        json!([format!("{tx1:#x}")]),
    )
    .await?;
    assert!(
        pending_tx1_before_growth.get("error").is_none(),
        "pending tx lookup should succeed: {pending_tx1_before_growth}"
    );
    assert_eq!(
        pending_tx1_before_growth["result"]["blockHash"].as_str(),
        Some(first_hash_hex.as_str()),
        "pending tx lookup should agree with the first surfaced pending blockHash"
    );

    let tx2 = client.alloy_emit_logs(contract_address, 2, 1).await;
    let second_log = timeout(SUBSCRIPTION_TIMEOUT, log_stream.next())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for second pending log"))?
        .ok_or_else(|| anyhow::anyhow!("log subscription closed before second event"))?;
    let second_hash = second_log
        .block_hash
        .expect("second pending log should include blockHash");
    assert_eq!(
        second_log.block_number,
        Some(first_block_number),
        "both logs should stay in the same pending block"
    );
    assert_eq!(
        second_hash, first_hash,
        "the surfaced pending blockHash should stay stable as the pending block grows"
    );

    let pending_tx1_after_growth = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getTransactionByHash",
        json!([format!("{tx1:#x}")]),
    )
    .await?;
    assert!(
        pending_tx1_after_growth.get("error").is_none(),
        "pending tx lookup after growth should succeed: {pending_tx1_after_growth}"
    );
    assert_eq!(
        pending_tx1_after_growth["result"]["blockHash"].as_str(),
        Some(first_hash_hex.as_str()),
        "pending tx lookup should keep the original surfaced blockHash while still pending"
    );

    rollup.resume_preferred_batches().await;
    let _ = client.wait_for_finalized_receipt(tx1).await;
    let _ = client.wait_for_finalized_receipt(tx2).await;

    let sealed_block = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!([hex_u64(first_block_number), false]),
    )
    .await?;
    assert!(
        sealed_block.get("error").is_none(),
        "sealed block lookup should succeed: {sealed_block}"
    );
    assert_eq!(
        sealed_block["result"]["hash"].as_str(),
        Some(first_hash_hex.as_str()),
        "sealed block should keep the original surfaced blockHash"
    );

    let sealed_tx1 = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getTransactionByHash",
        json!([format!("{tx1:#x}")]),
    )
    .await?;
    assert!(
        sealed_tx1.get("error").is_none(),
        "sealed tx lookup should succeed: {sealed_tx1}"
    );
    assert_eq!(
        sealed_tx1["result"]["blockHash"].as_str(),
        Some(first_hash_hex.as_str()),
        "sealed tx lookup should keep the original surfaced blockHash"
    );

    rollup.wait_for_rollup_height_advance_by(30).await;
    let by_hash = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByHash",
        json!([first_hash_hex, false]),
    )
    .await?;
    assert!(
        by_hash.get("error").is_none(),
        "the originally surfaced blockHash should remain queryable after the prune window: {by_hash}"
    );
    assert!(
        by_hash["result"].is_object(),
        "the originally surfaced blockHash should still resolve to a block object after the prune window"
    );

    Ok(())
}

/// RPC2-014: Explicit future numeric block selectors should return `null`.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_014_future_numeric_block_selector_returns_null() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    // Freeze the head so `latest` and `N + 1` are evaluated against the same chain state.
    rollup.pause_preferred_batches_and_wait().await?;

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

/// Hardhat #4: 0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65
/// Not in any genesis → starts with zero EVM balance.
const PAYMASTER_SIGNER_PRIV_KEY: &str =
    "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a";

/// RPC2-015: Paymaster-aware affordability checks in simulation and send paths.
#[tokio::test(flavor = "multi_thread")]
async fn rpc2_015_paymaster_estimate_send_affordability_consistency() -> anyhow::Result<()> {
    let rollup = setup_test_rollup_with_paymaster(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let paymaster_signer: PrivateKeySigner = PAYMASTER_SIGNER_PRIV_KEY.parse()?;
    let paymaster_address = paymaster_signer.address();
    let chain_id: U64 = ws_client.ws.request("eth_chainId", rpc_params![]).await?;
    let gas_limit = 1_000_000u64;
    let recipient = Address::repeat_byte(0x33);

    // ── Precondition: signer has zero EVM balance ──
    assert_eq!(
        ws_client.eth_get_balance(paymaster_address).await,
        U256::ZERO,
        "paymaster signer must start with zero EVM balance"
    );

    // ── Phase 1: zero-balance sender succeeds (paymaster covers gas) ──
    let base_fee: U256 = ws_client.ws.request("eth_gasPrice", rpc_params![]).await?;
    let base_fee_u128: u128 = base_fee.to::<u128>();
    assert!(base_fee_u128 > 0, "base fee should be non-zero");

    // Reasonable fee: above base fee, well below payer_balance / gas_limit
    let reasonable_max_fee = base_fee_u128 * 2;
    let reasonable_cost = (gas_limit as u128) * reasonable_max_fee;
    assert!(
        reasonable_cost < PAYER_SOV_BANK_BALANCE,
        "reasonable tx cost ({reasonable_cost}) must be below payer SOV balance ({PAYER_SOV_BANK_BALANCE})"
    );

    let http = Client::new();
    let estimate_request = json!({
        "from": paymaster_address,
        "to": recipient,
        "value": "0x0",
        "maxFeePerGas": hex_u128(reasonable_max_fee),
        "maxPriorityFeePerGas": "0x0"
    });
    let estimate_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        json!([estimate_request, "latest"]),
    )
    .await?;
    assert!(
        estimate_response.get("error").is_none(),
        "eth_estimateGas should succeed for zero-balance sender when paymaster covers gas: {estimate_response}"
    );
    assert!(
        estimate_response.get("result").is_some() && !estimate_response["result"].is_null(),
        "eth_estimateGas should return a gas estimate: {estimate_response}"
    );

    let nonce = tx_count(&ws_client, paymaster_address, "latest").await?;
    let raw_tx = raw_signed_eip1559(
        &paymaster_signer,
        chain_id.to::<u64>(),
        nonce,
        gas_limit,
        TxKind::Call(recipient),
        U256::ZERO,
        Bytes::new(),
        reasonable_max_fee,
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
        send_response.get("error").is_none(),
        "eth_sendRawTransaction should succeed for zero-balance sender when paymaster covers gas: {send_response}"
    );
    let tx_hash: B256 = rpc_result_hex(&send_response).parse()?;
    let receipt = ws_client.wait_for_receipt(tx_hash).await;
    assert!(
        receipt.status(),
        "transaction should succeed when paymaster covers gas"
    );

    // ── Phase 2: high declared fee does NOT cause false rejection ──
    // gas_limit * HIGH_MAX_FEE_PER_GAS = 1e6 * 1e12 = 1e18 > PAYER_SOV_BANK_BALANCE (5e15),
    // but the actual gas cost is gas_limit * rollup_gas_price ≈ 1e7 << PAYER_SOV_BANK_BALANCE.
    // After the cost-metric fix, both estimate and send use the actual gas cost, so they succeed.
    let nonce_after_phase_1 = tx_count(&ws_client, paymaster_address, "latest").await?;
    let high_fee_estimate_request = json!({
        "from": paymaster_address,
        "to": recipient,
        "value": "0x0",
        "gas": hex_u64(gas_limit),
        "maxFeePerGas": hex_u128(HIGH_MAX_FEE_PER_GAS),
        "maxPriorityFeePerGas": "0x0"
    });
    let high_fee_estimate = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        json!([high_fee_estimate_request, "latest"]),
    )
    .await?;
    assert!(
        high_fee_estimate.get("error").is_none(),
        "estimate should succeed with high declared fee when paymaster can afford actual gas cost: {high_fee_estimate}"
    );

    let high_fee_nonce = tx_count(&ws_client, paymaster_address, "latest").await?;
    let high_fee_raw_tx = raw_signed_eip1559(
        &paymaster_signer,
        chain_id.to::<u64>(),
        high_fee_nonce,
        gas_limit,
        TxKind::Call(recipient),
        U256::ZERO,
        Bytes::new(),
        HIGH_MAX_FEE_PER_GAS,
        0,
    )
    .await?;
    let high_fee_send = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendRawTransaction",
        json!([high_fee_raw_tx]),
    )
    .await?;
    assert!(
        high_fee_send.get("error").is_none(),
        "send should succeed with high declared fee when paymaster can afford actual gas cost: {high_fee_send}"
    );

    let nonce_after = tx_count(&ws_client, paymaster_address, "latest").await?;
    assert_eq!(
        nonce_after,
        nonce_after_phase_1 + 1,
        "successful phase-2 tx should advance nonce by 1"
    );

    Ok(())
}
