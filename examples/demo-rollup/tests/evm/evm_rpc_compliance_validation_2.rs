use crate::evm::evm_test_helper::{
    alloy_ws_client, create_simple_storage_client, deploy_contract_check, hex_u128, hex_u64,
    parse_hex_u128, parse_hex_u64, raw_signed_eip1559, rpc_call, rpc_error_code_from_response,
    rpc_error_message, rpc_error_object, rpc_result_hex, setup_test_rollup,
    setup_test_rollup_with_paymaster, setup_with_simple_storage, tx_count, EVM_EXTENSION,
    HIGH_MAX_FEE_PER_GAS, HIGH_PRIORITY_FEE_PER_GAS, PAYER_SOV_BANK_BALANCE, SENDER_PRIV_KEY,
};
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256, U64};
use alloy_provider::Provider;
use alloy_rpc_types_eth::Filter;
use alloy_rpc_types_trace::geth::GethTrace;
use futures::StreamExt;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use reqwest::Client;
use serde_json::json;
use tokio::time::{timeout, Duration};

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
const FEE_CAP_TOO_LOW_ERROR: &str = "max fee per gas less than block base fee";
const EMPTY_WITHDRAWALS_ROOT: &str =
    "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421";
const SUBSCRIPTION_TIMEOUT: Duration = Duration::from_secs(10);

/// RPC2-001: Fee-cap admission consistency across simulation and submission.
///
/// **Discrepancy**: Ethereum uniformly rejects `maxFeePerGas < baseFee` across
/// `eth_estimateGas`, `eth_call`, and `eth_sendRawTransaction`. The SDK can let
/// simulation accept a below-basefee tx while the send path rejects it (or vice
/// versa), producing a false-positive preflight result.
///
/// **Impact**: Wallets and SDKs (MetaMask, ethers.js, viem) that preflight via
/// `eth_estimateGas` before signing can show "estimate OK" then fail at send time.
///
/// **Test strategy**: Submits the same below-basefee transaction to all three
/// endpoints and asserts they return identical error codes.
///
/// Overlap note: earlier low-fee-cap rejection coverage lives in
/// `evm_call_fee_fields.rs::{eth_call_rejects_below_base_fee_with_max_fee_per_gas, eth_create_access_list_rejects_below_base_fee_with_max_fee_per_gas}`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known discrepancy: will be fixed in the follow up"]
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
    let call_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_call",
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
        call_response.get("error").is_some(),
        "eth_call should reject maxFeePerGas below base fee: {call_response}"
    );
    assert!(
        send_response.get("error").is_some(),
        "send should reject maxFeePerGas below base fee: {send_response}"
    );
    assert!(
        estimate_response.get("result").is_none() || estimate_response["result"].is_null(),
        "estimate should not return a result on fee-cap rejection: {estimate_response}"
    );
    assert!(
        call_response.get("result").is_none() || call_response["result"].is_null(),
        "eth_call should not return a result on fee-cap rejection: {call_response}"
    );
    assert!(
        send_response.get("result").is_none() || send_response["result"].is_null(),
        "send should not return a result on fee-cap rejection: {send_response}"
    );
    assert_eq!(
        rpc_error_code_from_response(&estimate_response, "eth_estimateGas"),
        rpc_error_code_from_response(&send_response, "eth_sendRawTransaction"),
        "estimate and send should reject with the same JSON-RPC error class"
    );
    assert_eq!(
        rpc_error_code_from_response(&call_response, "eth_call"),
        rpc_error_code_from_response(&send_response, "eth_sendRawTransaction"),
        "eth_call and send should reject with the same JSON-RPC error class"
    );
    assert!(
        rpc_error_message(rpc_error_object(&estimate_response, "eth_estimateGas"))
            .contains(FEE_CAP_TOO_LOW_ERROR),
        "estimate should reject for the exact below-base-fee reason: {estimate_response}"
    );
    assert!(
        rpc_error_message(rpc_error_object(&call_response, "eth_call"))
            .contains(FEE_CAP_TOO_LOW_ERROR),
        "eth_call should reject for the exact below-base-fee reason: {call_response}"
    );
    assert!(
        rpc_error_message(rpc_error_object(&send_response, "eth_sendRawTransaction"))
            .contains(FEE_CAP_TOO_LOW_ERROR),
        "send should reject for the exact below-base-fee reason: {send_response}"
    );
    assert_eq!(
        tx_count(&ws_client, signer.address(), "latest").await?,
        nonce,
        "failed raw send must not advance sender nonce"
    );

    Ok(())
}

/// RPC2-002: Affordability check consistency between estimation and send path.
///
/// **Discrepancy**: `eth_estimateGas` affordability uses the request fee cap,
/// while the submission path charges via the rollup pricing model. A sender
/// whose balance falls in the window `gas_limit * rollup_price < B < gas_limit *
/// maxFeePerGas` can pass estimation but fail submission, or vice versa.
///
/// **Impact**: Wallets treating estimation failure as a hard gate can block valid
/// transactions (false negative before send).
///
/// **Test strategy**: Funds a signer into the affordability window and submits
/// via both `eth_estimateGas` and `eth_sendRawTransaction`, asserting both
/// reject with the same insufficient-funds error. Not an `eth_call` parity
/// test — local call/base-fee semantics live in `evm_call_fee_fields.rs`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known discrepancy: will be fixed in the follow up"]
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

/// RPC2-003 (gasleft probe): Omitted-gas `eth_call` should default to the tx
/// gas cap, not the block gas limit.
///
/// **Discrepancy**: When `gas` is omitted, `eth_call` can execute against the
/// block gas limit (1B) instead of the 30M tx gas cap. This gives simulation
/// more headroom than a real transaction would have.
///
/// **Impact**: `callStatic` and gas-estimation flows can succeed on workloads
/// that are unreachable once the tx is sent under the real cap.
///
/// **Test strategy**: Deploys a `gasleft()` contract and compares the gas
/// available in an omitted-gas `eth_call` vs an explicit 30M-gas call. Asserts
/// the two values are within 500K of each other (not orders of magnitude apart
/// as they would be if the block gas limit were used).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known discrepancy: will be fixed in the follow up"]
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

/// RPC2-003 (end-to-end burnGas): Omitted-gas simulation agrees with real tx
/// cap on workload classification.
///
/// **Discrepancy**: Same as the gasleft probe above — simulation may run under a
/// broader gas context than the real tx cap allows.
///
/// **Impact**: Wallets preview a workload as feasible that then fails on-chain.
///
/// **Test strategy**: Binary-searches for a `burnGas(N)` workload that fails
/// under the real 30M tx cap, then verifies 4-way consistency: (1) omitted-gas
/// `eth_call` also fails, (2) omitted-gas `eth_estimateGas` also rejects,
/// (3) a real tx with explicit 30M gas also fails, and (4) the same workload
/// succeeds when given enough gas headroom below the boundary.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known discrepancy: will be fixed in the follow up"]
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
    let simulation_max_fee_per_gas = DEFAULT_MAX_FEE_PER_GAS;
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

/// RPC2-004: `eth_estimateGas` units track receipt `gasUsed` (internal
/// consistency only).
///
/// **Discrepancy**: `eth_estimateGas` returns sovereign-metered gas with rollup
/// overheads and margins, not raw EVM execution gas. Wallets see unexpectedly
/// large `gasLimit` values compared to mainnet for equivalent operations.
///
/// **Impact**: Gas-limit displays, fee-estimation UIs, and gas-comparison logic
/// in wallets and SDKs can be misleading or trigger user-facing warnings.
///
/// **Test strategy**: Compares `eth_estimateGas` against the receipt `gasUsed`
/// for the same contract call, asserting the two are within 10K of each other.
/// **Note (moderate faithfulness)**: this validates estimate-receipt *internal
/// consistency*, not whether either value matches Ethereum's raw EVM gas. Both
/// values use sovereign metering, so the test passes even if both are inflated
/// relative to mainnet. The full finding's claim about absolute unit divergence
/// is not directly exercised here.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known discrepancy: will be fixed in the follow up"]
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

/// RPC2-005: Receipt fee fields reconcile exactly with sender balance delta.
///
/// **Discrepancy**: `gasUsed * effectiveGasPrice` in the receipt can diverge
/// from the actual fee charged (observable as sender balance delta). Historical
/// receipts and the sovereign pricing model can produce fee fields that do not
/// tell a coherent cost story.
///
/// **Impact**: Explorers, accounting tools, and wallet history views that derive
/// cost from receipt fields show fees that do not match the sender's observed
/// balance change.
///
/// **Test strategy**: Sends a value transfer, records balance before/after, and
/// asserts `balance_before - balance_after == value + gasUsed *
/// effectiveGasPrice`.
///
/// Overlap note: receipt fee reconciliation is also covered by
/// `evm_rpc_compliance_validation.rs::rpc_008_receipt_fee_fields_match_balance_delta`
/// and `sov-evm/tests/integration/transactions.rs::test_block_receipt_fee_matches_balance_delta`.
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

/// RPC2-006: `eth_feeHistory(block_count=0)` should return an empty result, not
/// an error.
///
/// **Discrepancy**: Ethereum returns an empty-shaped success result for
/// `block_count=0`. The SDK returns `-32602` ("block_count must be greater
/// than 0"), breaking defensive clients that probe this edge case.
///
/// **Impact**: Low severity. Most wallets request one or more blocks, but
/// generic or spec-surface clients can fail unexpectedly.
///
/// **Test strategy**: Calls `eth_feeHistory(0, "latest", [])` and asserts the
/// response is a success with correct empty-shape fields (empty `baseFeePerGas`
/// array, `oldestBlock`, no reward array).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known discrepancy: will be fixed in the follow up"]
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
        response["result"]["baseFeePerGas"],
        json!([]),
        "baseFeePerGas should be exactly [] for zero-block feeHistory"
    );
    assert_eq!(
        response["result"]["baseFeePerGas"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        0
    );
    assert_eq!(
        response["result"]["gasUsedRatio"],
        json!([]),
        "gasUsedRatio should be exactly [] for zero-block feeHistory"
    );
    assert_eq!(
        response["result"]["gasUsedRatio"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        0
    );
    assert!(
        response["result"].get("reward").is_none() || response["result"]["reward"].is_null(),
        "reward should be omitted for zero-block feeHistory with empty percentiles: {response}"
    );

    Ok(())
}

/// RPC2-007: `eth_feeHistory.reward` percentiles should reflect actual tips.
///
/// **Discrepancy**: Reward percentiles are always returned as zero regardless of
/// the `maxPriorityFeePerGas` paid by transactions in the block. EIP-1559
/// wallets that extract tip signal from `eth_feeHistory.reward` get no
/// differentiation between fee tiers.
///
/// **Impact**: Fee estimation UIs (MetaMask, ethers.js, viem) collapse low,
/// medium, and high tip tiers into the same value, degrading fee suggestion
/// quality.
///
/// **Test strategy**: Sends a finalized EIP-1559 tx with non-zero
/// `maxPriorityFeePerGas`, queries `eth_feeHistory` for that block's 50th
/// percentile reward, and asserts it is non-zero.
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

/// RPC2-008: Post-Cancun blocks should include `withdrawals: []` and a
/// canonical empty `withdrawalsRoot`.
///
/// **Discrepancy**: Block responses use `withdrawals: null` (pre-Shanghai shape)
/// instead of `[]`, and omit the canonical empty withdrawals root
/// (`0x56e81f...b421`). Strict decoders interpret this as a hardfork-era
/// mismatch.
///
/// **Impact**: Block decoders and explorers can reject or flag blocks as
/// internally inconsistent with expected Cancun-era schema.
///
/// **Test strategy**: Fetches `eth_getBlockByNumber("latest", false)` and
/// asserts `withdrawals == []` and `withdrawalsRoot` equals the canonical empty
/// root.
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
///
/// **Discrepancy**: The same absent block hash can yield an RPC error on one
/// endpoint and `null` on another. Ethereum uniformly returns `result: null` for
/// any unknown hash.
///
/// **Impact**: Retry, pruning, and not-found handling in providers and indexers
/// breaks when callers cannot rely on one consistent response shape for missing
/// hashes.
///
/// **Test strategy**: Queries `eth_getBlockByHash` and
/// `eth_getBlockTransactionCountByHash` with a fabricated never-existed hash
/// (`0x42` repeated) and asserts both return `result: null` without RPC errors.
/// **Note (moderate faithfulness)**: the finding's strongest repro uses pruned
/// synthetic hashes (see RPC2-013), not fabricated ones. This test still
/// validates the basic cross-endpoint consistency requirement.
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
///
/// **Discrepancy**: Only `callTracer` is supported. Calling
/// `debug_traceTransaction` without specifying a tracer returns
/// `{"code":-32603,"message":"unsupported tracer"}` instead of the default
/// struct-log trace Geth produces.
///
/// **Impact**: Foundry, Hardhat, Tenderly, and custom debugging flows that rely
/// on the default tracer path fail immediately.
///
/// **Test strategy**: Sends a tx, then calls `debug_traceTransaction` with an
/// empty params object (no tracer specified) and asserts the result is a valid
/// `GethTrace::Default`.
/// Overlap note: default tracer coverage also exists in
/// `evm_tracing.rs::debug_trace_block_by_number_default_tracer` and
/// `sov-evm/tests/integration/trace.rs`.
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
///
/// **Discrepancy**: The subscription type is unsupported. `newHeads` and `logs`
/// exist, but pending transaction subscription does not, even though it is part
/// of the standard Ethereum subscription surface.
///
/// **Impact**: Mempool monitoring, pending-tx dashboards, and tooling that
/// assumes the standard pending subscription cannot operate without a custom
/// fallback.
///
/// **Test strategy**: Pauses batch production, subscribes to
/// `newPendingTransactions`, sends a tx, and asserts the subscription emits the
/// pending tx hash within a timeout.
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

/// RPC2-012: `safe` and `finalized` tags should match `latest` on an
/// instant-finality chain.
///
/// **Discrepancy**: `safe` and `finalized` can resolve behind `latest` under
/// finality lag. On a chain with near-instant finality this creates unexpected
/// staleness for clients polling these tags.
///
/// **Impact**: Clients polling `safe` or `finalized` see unexpectedly stale
/// state relative to `latest`, confusing finality-aware UX and chain-state
/// polling logic.
///
/// **Test strategy**: Runs on the default instant-finality configuration,
/// queries all three tags (`latest`, `safe`, `finalized`) via
/// `eth_getBlockByNumber`, and asserts all three return the same block number.
/// Overlap note: block-tag consistency is also covered by
/// `evm_block_by_number_hash.rs::{test_block_tags_earliest_safe_finalized, test_block_number_consistency}`.
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

/// RPC2-013: Synthetic pending `blockHash` should remain stable and resolvable
/// across the pending-to-sealed-to-pruned lifecycle.
///
/// **Discrepancy**: Pending logs and tx lookups expose synthetic `blockHash`
/// values that change once the block is sealed and can become unresolvable
/// after pruning. Ethereum expects that once a hash is surfaced, it remains a
/// stable cross-RPC identifier.
///
/// **Impact**: Indexers, explorers, and log-correlation tooling fail to join
/// pending and sealed data by hash or encounter stale hashes that no longer
/// resolve.
///
/// **Test strategy**: Exercises a 3-phase lifecycle — (1) captures the pending
/// `blockHash` from a log subscription, (2) seals the block and verifies the
/// hash resolves to the same block, (3) advances past the pruning window and
/// checks the hash still resolves. Strongest multi-source finding (confirmed by
/// all three audit agents).
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
///
/// **Discrepancy**: A request for block `N+1` (one beyond the current height)
/// can alias to the current block `N` instead of returning `null`. This violates
/// the `last_seen + 1` polling model indexers rely on.
///
/// **Impact**: Indexers and provider poll loops ingest incorrect block-to-height
/// mappings instead of receiving a clean future-block miss.
///
/// **Test strategy**: Fetches `latest` to get the current block number, then
/// queries `eth_getBlockByNumber` with `N+1` and asserts the result is `null`.
///
/// Overlap note: missing-block null semantics are also covered by
/// `evm_block_by_number_hash.rs::test_nonexistent_block_returns_none`.
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
///
/// **Discrepancy**: When a paymaster covers gas, `eth_estimateGas` and
/// `eth_sendRawTransaction` should check affordability against the paymaster's
/// SOV bank balance, not the sender's (possibly zero) EVM balance. Currently
/// the RPC paths are not paymaster-aware.
///
/// **Impact**: Zero-balance senders backed by a paymaster are incorrectly
/// rejected by estimation, even though the signed transaction would succeed
/// on-chain.
///
/// **Test strategy**: Two phases — (1) confirms a zero-balance sender can
/// transact when a paymaster is configured and willing to cover gas, (2) mirrors
/// the rpc2_002 affordability window test but checks against the paymaster's SOV
/// bank balance instead of the sender's EVM balance.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Paymaster-aware RPC affordability checks not yet implemented"]
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
        "gas": hex_u64(gas_limit),
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

    // ── Phase 2: affordability window against paymaster balance ──
    // gas_limit * HIGH_MAX_FEE_PER_GAS = 1e6 * 1e12 = 1e18 > PAYER_SOV_BANK_BALANCE (5e15)
    // gas_limit * base_fee ≈ 1e7 < PAYER_SOV_BANK_BALANCE
    let estimate_ceiling = (gas_limit as u128) * HIGH_MAX_FEE_PER_GAS;
    assert!(
        estimate_ceiling > PAYER_SOV_BANK_BALANCE,
        "estimate ceiling ({estimate_ceiling}) must exceed payer SOV balance ({PAYER_SOV_BANK_BALANCE})"
    );
    let send_floor = (gas_limit as u128) * base_fee_u128;
    assert!(
        send_floor < PAYER_SOV_BANK_BALANCE,
        "send floor ({send_floor}) must be below payer SOV balance ({PAYER_SOV_BANK_BALANCE})"
    );

    let nonce_before = tx_count(&ws_client, paymaster_address, "latest").await?;
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
    assert_eq!(
        rpc_error_code_from_response(&high_fee_estimate, "eth_estimateGas"),
        -32003,
        "estimate should reject with transaction-rejected error when paymaster can't afford gas: {high_fee_estimate}"
    );
    assert!(
        rpc_error_message(rpc_error_object(&high_fee_estimate, "eth_estimateGas"))
            .contains(INSUFFICIENT_FUNDS_FOR_GAS_ERROR),
        "estimate should report insufficient-funds against paymaster balance: {high_fee_estimate}"
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
    assert_eq!(
        rpc_error_code_from_response(&high_fee_send, "eth_sendRawTransaction"),
        rpc_error_code_from_response(&high_fee_estimate, "eth_estimateGas"),
        "estimate and send should reject with the same JSON-RPC error class"
    );
    assert!(
        rpc_error_message(rpc_error_object(&high_fee_send, "eth_sendRawTransaction"))
            .contains(INSUFFICIENT_FUNDS_FOR_GAS_ERROR),
        "send should report insufficient-funds against paymaster balance: {high_fee_send}"
    );

    let nonce_after = tx_count(&ws_client, paymaster_address, "latest").await?;
    assert_eq!(
        nonce_after,
        nonce_before + 1,
        "only the successful phase-1 tx should advance nonce; failed phase-2 tx must not"
    );

    Ok(())
}
