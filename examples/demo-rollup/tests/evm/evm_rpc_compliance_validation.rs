use alloy::rpc::types::{
    state::{AccountOverride, StateOverride},
    BlockOverrides,
};
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256, U64};
use alloy_provider::{ext::DebugApi, Provider};
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter, TransactionRequest};
use alloy_rpc_types_trace::geth::{CallConfig, GethDebugTracingOptions, GethTrace, TraceResult};
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::rpc_params;
use reqwest::Client;
use serde_json::{json, Value};
use sov_evm_test_utils::SimpleStorage;
use sov_rpc_eth_types::FilterWithCursor;
use tokio::time::{timeout, Duration};

use crate::evm::evm_test_helper::{
    alloy_client, create_simple_storage_client, deploy_contract_check,
    finalized_block_number_and_hash, hash_selector, hex_u64, hex_word_u64, parse_hex_u64,
    poll_until, raw_signed_eip1559, rpc_call, rpc_error_code, rpc_error_data_str,
    rpc_error_message, rpc_error_object, rpc_result_hex, setup_test_rollup,
    setup_with_simple_storage, tx_count, EVM_EXTENSION, INVALID_PARAMS_CODE, SENDER_PRIV_KEY,
};

const MAX_FEE_PER_GAS: u128 = 1_000_000_000;
const MAX_PRIORITY_FEE_PER_GAS: u128 = 1;
const SUBSCRIPTION_TIMEOUT: Duration = Duration::from_secs(10);

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

// RPC-001
#[tokio::test(flavor = "multi_thread")]
async fn rpc_001_block_number_matches_latest_with_pending_head() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let provider = alloy_client(rollup.http_addr);
    let sealed_head_before = finalized_block_number_and_hash(&client).await.0;

    rollup.pause_preferred_batches_and_wait().await?;
    let tx_hash = client.send_eth(Address::ZERO, U256::from(1u64)).await;
    let pending_receipt = client.wait_for_receipt(tx_hash).await;
    let expected_head = pending_receipt
        .block_number
        .expect("pending receipt should include block number");

    let block_number = provider.get_block_number().await?;
    let latest_block = client
        .eth_get_block_by_number(Some("latest".to_string()))
        .await;

    assert!(
        expected_head > sealed_head_before,
        "pending head precondition was not exercised"
    );
    assert_eq!(
        block_number, latest_block.header.number,
        "eth_blockNumber must match eth_getBlockByNumber(latest).number under a real pending head"
    );
    assert_eq!(
        latest_block.header.number, expected_head,
        "latest should resolve to the pending head that surfaced the pending receipt"
    );
    assert!(
        latest_block
            .transactions
            .hashes()
            .any(|hash| hash == tx_hash),
        "latest block must contain the pending transaction under test"
    );

    Ok(())
}

// RPC-002
#[tokio::test(flavor = "multi_thread")]
async fn rpc_002_block_pinned_reads_exclude_pending_state_at_current_head() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let sender = client.address();
    let receiver = Address::repeat_byte(0xbb);
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let head_number = client.block_number().await;
    let head_hash = client
        .eth_get_block_by_number(Some(hex_u64(head_number)))
        .await
        .header
        .hash;
    let finalized_before_pause = finalized_block_number_and_hash(&client).await;
    let head_selector = json!(hex_u64(head_number));

    let balance_at = |address: Address, block: Value| {
        let client = &client;
        async move {
            let balance: U256 = client
                .ws
                .request("eth_getBalance", rpc_params![address, block])
                .await?;
            anyhow::Ok(balance)
        }
    };
    let storage_at = |block: Value| {
        let client = &client;
        async move {
            let value: B256 = client
                .ws
                .request("eth_getStorageAt", rpc_params![contract, U256::ZERO, block])
                .await?;
            anyhow::Ok(U256::from_be_slice(value.as_slice()))
        }
    };
    let call_at = |tx: &TransactionRequest, block: Value| {
        let client = &client;
        let mut tx = tx.clone();
        tx.nonce = None;
        async move {
            let output: Bytes = client
                .ws
                .request("eth_call", rpc_params![tx, block])
                .await?;
            anyhow::Ok(U256::from_be_slice(output.as_ref()))
        }
    };
    let estimate_at = |tx: &TransactionRequest, block: Value| {
        let client = &client;
        let mut tx = tx.clone();
        tx.nonce = None;
        async move {
            let estimate: U256 = client
                .ws
                .request("eth_estimateGas", rpc_params![tx, block])
                .await?;
            anyhow::Ok(estimate)
        }
    };

    let sealed_nonce = tx_count(&client, sender, head_selector.clone()).await?;
    let sealed_balance = balance_at(receiver, head_selector.clone()).await?;
    let get_tx = client.make_tx(Some(contract), Some(client.contract.get()));
    let sealed_storage = storage_at(head_selector.clone()).await?;
    let sealed_call = call_at(&get_tx, head_selector.clone()).await?;

    let estimate_tx = client.make_tx(Some(contract), Some(client.contract.set(0x5678)));
    let sealed_estimate = estimate_at(&estimate_tx, head_selector.clone()).await?;

    assert_eq!(sealed_balance, U256::ZERO, "receiver should start unfunded");
    assert_eq!(
        sealed_storage,
        U256::ZERO,
        "contract slot should start at zero"
    );
    assert_eq!(
        sealed_call,
        U256::ZERO,
        "eth_call should observe zeroed storage"
    );

    rollup.pause_preferred_batches_and_wait().await?;
    let transfer_amount = U256::from(0x1_0000_0000u64);
    let transfer_hash = client.send_eth(receiver, transfer_amount).await;
    client.wait_for_receipt(transfer_hash).await;
    let storage_hash = client.set_value(contract, 0x1234).await;
    client.wait_for_receipt(storage_hash).await;

    let hash_pinned = hash_selector(head_hash);
    assert_eq!(
        tx_count(&client, sender, head_selector.clone()).await?,
        sealed_nonce,
        "numeric block pin must stay sealed even after pending writes"
    );
    assert_eq!(
        tx_count(&client, sender, hash_pinned.clone()).await?,
        sealed_nonce,
        "hash block pin must stay sealed even after pending writes"
    );
    assert_eq!(
        balance_at(receiver, head_selector.clone()).await?,
        sealed_balance,
        "pinned balance by number must not leak pending transfer"
    );
    assert_eq!(
        balance_at(receiver, hash_pinned.clone()).await?,
        sealed_balance,
        "pinned balance by hash must not leak pending transfer"
    );
    assert_eq!(
        storage_at(head_selector.clone()).await?,
        sealed_storage,
        "pinned storage by number must not leak pending contract mutation"
    );
    assert_eq!(
        storage_at(hash_pinned.clone()).await?,
        sealed_storage,
        "pinned storage by hash must not leak pending contract mutation"
    );
    assert_eq!(
        call_at(&get_tx, head_selector.clone()).await?,
        sealed_call,
        "pinned eth_call by number must not leak pending contract mutation"
    );
    assert_eq!(
        call_at(&get_tx, hash_pinned.clone()).await?,
        sealed_call,
        "pinned eth_call by hash must not leak pending contract mutation"
    );
    assert_eq!(
        estimate_at(&estimate_tx, head_selector.clone()).await?,
        sealed_estimate,
        "pinned estimate by number must match the sealed-state baseline"
    );
    assert_eq!(
        estimate_at(&estimate_tx, hash_pinned).await?,
        sealed_estimate,
        "pinned estimate by hash must match the sealed-state baseline"
    );

    let latest_nonce = tx_count(&client, sender, "latest").await?;
    let pending_nonce = tx_count(&client, sender, "pending").await?;
    assert_eq!(latest_nonce, sealed_nonce + 2);
    assert_eq!(pending_nonce, latest_nonce);
    assert_eq!(
        balance_at(receiver, json!("latest")).await?,
        transfer_amount,
        "latest balance must reflect the pending transfer"
    );
    assert_eq!(
        balance_at(receiver, json!("pending")).await?,
        transfer_amount,
        "pending balance must reflect the pending transfer"
    );
    assert_eq!(
        storage_at(json!("latest")).await?,
        U256::from(0x1234u64),
        "latest storage must reflect the pending contract mutation"
    );
    assert_eq!(
        storage_at(json!("pending")).await?,
        U256::from(0x1234u64),
        "pending storage must reflect the pending contract mutation"
    );
    assert_eq!(
        call_at(&get_tx, json!("latest")).await?,
        U256::from(0x1234u64),
        "latest eth_call must observe pending contract state"
    );
    assert_eq!(
        call_at(&get_tx, json!("pending")).await?,
        U256::from(0x1234u64),
        "pending eth_call must observe pending contract state"
    );
    let pending_estimate = estimate_at(&estimate_tx, json!("pending")).await?;
    assert_ne!(
        pending_estimate, sealed_estimate,
        "pending estimate should differ once the storage slot becomes non-zero in current state"
    );
    assert_eq!(
        estimate_at(&estimate_tx, json!("latest")).await?,
        pending_estimate,
        "latest estimate should match pending estimate in pending-inclusive semantics"
    );
    assert_eq!(
        finalized_block_number_and_hash(&client).await,
        finalized_before_pause,
        "finalized head should stay fixed while preferred batches are paused"
    );

    Ok(())
}

// RPC-003
#[tokio::test(flavor = "multi_thread")]
async fn rpc_003_estimate_gas_uses_account_nonce_when_nonce_omitted() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let provider = alloy_client(rollup.http_addr);
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let sender = client.address();

    let first_deploy = client
        .deploy_contract()
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let first_receipt = client.wait_for_receipt(first_deploy).await;
    assert!(
        first_receipt.contract_address.is_some(),
        "first deployment should consume CREATE nonce 0"
    );

    let next_nonce = tx_count(&client, sender, "latest").await?;
    assert_eq!(next_nonce, 1, "sender should now be at nonce 1");

    let deploy = SimpleStorage::deploy_builder(provider.clone());
    let deploy_request = deploy.into_transaction_request();
    let bytecode = deploy_request.input.into_input().unwrap_or_default();

    let request_omitted_nonce = json!({
        "from": sender,
        "data": bytecode,
        "maxFeePerGas": hex_u64(MAX_FEE_PER_GAS as u64),
        "maxPriorityFeePerGas": "0x1"
    });
    let request_current_nonce = json!({
        "from": sender,
        "data": bytecode,
        "maxFeePerGas": hex_u64(MAX_FEE_PER_GAS as u64),
        "maxPriorityFeePerGas": "0x1",
        "nonce": hex_u64(next_nonce)
    });
    let request_stale_nonce = json!({
        "from": sender,
        "data": bytecode,
        "maxFeePerGas": hex_u64(MAX_FEE_PER_GAS as u64),
        "maxPriorityFeePerGas": "0x1",
        "nonce": "0x0"
    });

    let estimate_omitted: U64 = client
        .ws
        .request(
            "eth_estimateGas",
            rpc_params![request_omitted_nonce.clone(), "latest"],
        )
        .await?;
    let estimate_current: U64 = client
        .ws
        .request(
            "eth_estimateGas",
            rpc_params![request_current_nonce.clone(), "latest"],
        )
        .await?;

    assert_eq!(
        estimate_omitted, estimate_current,
        "omitted nonce must simulate with the current account nonce, not default back to zero"
    );

    let http = Client::new();
    let stale_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        json!([request_stale_nonce, "latest"]),
    )
    .await?;
    assert!(
        stale_response.get("error").is_some(),
        "explicit stale nonce should be rejected after the first deployment: {stale_response}"
    );
    assert!(
        stale_response.get("result").is_none() || stale_response["result"].is_null(),
        "eth_estimateGas should not return a result when stale nonce is rejected: {stale_response}"
    );
    assert!(
        rpc_error_message(rpc_error_object(&stale_response, "eth_estimateGas")).contains("nonce"),
        "stale nonce rejection should mention nonce semantics: {stale_response}"
    );

    Ok(())
}

// RPC-004
#[tokio::test(flavor = "multi_thread")]
async fn rpc_004_eth_call_applies_request_context_semantics() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let provider = alloy_client(rollup.http_addr);
    let sender = client.address();
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let set_tx_hash = client.set_value(contract, 1).await;
    client.wait_for_receipt(set_tx_hash).await;

    let get_tx = client.make_tx(Some(contract), Some(client.contract.get()));
    let baseline: Bytes = client
        .ws
        .request("eth_call", rpc_params![&get_tx, "latest"])
        .await?;
    assert_eq!(
        U256::from_be_slice(baseline.as_ref()),
        U256::from(1u64),
        "baseline eth_call must observe the sealed contract state"
    );

    let mut storage_overrides = StateOverride::default();
    storage_overrides.insert(
        contract,
        AccountOverride::default().with_state_diff([(
            B256::ZERO,
            B256::from(U256::from(42u64).to_be_bytes::<32>()),
        )]),
    );
    let overridden: Bytes = client
        .ws
        .request(
            "eth_call",
            rpc_params![&get_tx, "latest", storage_overrides.clone()],
        )
        .await?;
    assert_eq!(
        U256::from_be_slice(overridden.as_ref()),
        U256::from(42u64),
        "stateDiff must override storage for eth_call exactly"
    );

    let target = Address::repeat_byte(0x55);
    let block_number_request = TransactionRequest::default()
        .from(sender)
        .to(target)
        .gas_limit(300_000)
        .max_fee_per_gas(MAX_FEE_PER_GAS)
        .max_priority_fee_per_gas(MAX_PRIORITY_FEE_PER_GAS);
    let mut block_number_code = StateOverride::default();
    block_number_code.insert(
        target,
        AccountOverride::default().with_code(Bytes::from(vec![
            0x43, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
        ])),
    );
    let latest_number = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .expect("latest block should exist")
        .header
        .number;
    let block_number_baseline: Bytes = client
        .ws
        .request(
            "eth_call",
            rpc_params![
                block_number_request.clone(),
                "latest",
                block_number_code.clone()
            ],
        )
        .await?;
    assert_eq!(
        U256::from_be_slice(block_number_baseline.as_ref()),
        U256::from(latest_number),
        "block.number opcode must observe the real latest block number without overrides"
    );

    let block_overrides = BlockOverrides::default()
        .with_number(U256::from(77u64))
        .with_base_fee(U256::from(1_234u64));
    let block_number_overridden: Bytes = client
        .ws
        .request(
            "eth_call",
            rpc_params![
                block_number_request.clone(),
                "latest",
                block_number_code,
                block_overrides.clone()
            ],
        )
        .await?;
    assert_eq!(
        U256::from_be_slice(block_number_overridden.as_ref()),
        U256::from(77u64),
        "blockOverrides.number must change eth_call block context"
    );

    let base_fee_target = Address::repeat_byte(0x56);
    let base_fee_request = TransactionRequest::default()
        .from(sender)
        .to(base_fee_target)
        .gas_limit(300_000)
        .max_fee_per_gas(MAX_FEE_PER_GAS)
        .max_priority_fee_per_gas(MAX_PRIORITY_FEE_PER_GAS);
    let mut base_fee_code = StateOverride::default();
    base_fee_code.insert(
        base_fee_target,
        AccountOverride::default().with_code(Bytes::from(vec![
            0x48, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
        ])),
    );
    let base_fee_overridden: Bytes = client
        .ws
        .request(
            "eth_call",
            rpc_params![
                base_fee_request,
                "latest",
                base_fee_code,
                block_overrides.clone()
            ],
        )
        .await?;
    assert_eq!(
        U256::from_be_slice(base_fee_overridden.as_ref()),
        U256::from(1_234u64),
        "blockOverrides.baseFee must change eth_call BASEFEE context"
    );

    let estimate_target = Address::repeat_byte(0x57);
    let estimate_request = TransactionRequest::default()
        .from(sender)
        .to(estimate_target)
        .gas_limit(300_000)
        .max_fee_per_gas(MAX_FEE_PER_GAS)
        .max_priority_fee_per_gas(MAX_PRIORITY_FEE_PER_GAS);
    let mut estimate_state = StateOverride::default();
    estimate_state.insert(
        estimate_target,
        AccountOverride::default().with_code(Bytes::from(vec![
            0x43, 0x60, 77, 0x14, 0x60, 0x0c, 0x57, 0x60, 0x00, 0x60, 0x00, 0xfd, 0x5b, 0x00,
        ])),
    );
    let http = Client::new();
    let estimate_without_override = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        json!([
            serde_json::to_value(&estimate_request)?,
            "latest",
            serde_json::to_value(&estimate_state)?
        ]),
    )
    .await?;
    assert!(
        estimate_without_override.get("error").is_some(),
        "estimate should fail when the required block override is missing: {estimate_without_override}"
    );
    assert!(
        estimate_without_override.get("result").is_none()
            || estimate_without_override["result"].is_null(),
        "eth_estimateGas should not return a result when blockOverrides.number is required: {estimate_without_override}"
    );

    let estimate_with_override = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        json!([
            serde_json::to_value(&estimate_request)?,
            "latest",
            serde_json::to_value(&estimate_state)?,
            serde_json::to_value(&block_overrides)?
        ]),
    )
    .await?;
    assert!(
        estimate_with_override.get("error").is_none(),
        "estimate should succeed once blockOverrides.number is applied: {estimate_with_override}"
    );
    assert!(
        parse_hex_u64(
            estimate_with_override["result"]
                .as_str()
                .expect("estimate result should be a hex quantity")
        ) > 0,
        "successful estimate must return a positive gas quantity"
    );

    let access_list_request = TransactionRequest::default()
        .from(sender)
        .to(contract)
        .gas_limit(300_000)
        .max_fee_per_gas(MAX_FEE_PER_GAS)
        .max_priority_fee_per_gas(MAX_PRIORITY_FEE_PER_GAS)
        .input(client.contract.set(2).into());
    let access_list_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_createAccessList",
        json!([serde_json::to_value(&access_list_request)?, "latest"]),
    )
    .await?;
    assert!(
        access_list_response.get("error").is_none(),
        "typed EIP-1559 request should be accepted by eth_createAccessList: {access_list_response}"
    );
    assert!(
        access_list_response["result"]["accessList"].is_array(),
        "eth_createAccessList should return an accessList array: {access_list_response}"
    );
    assert!(
        access_list_response["result"]["gasUsed"].is_string(),
        "eth_createAccessList should return gasUsed as a hex quantity string: {access_list_response}"
    );

    Ok(())
}

// RPC-005
#[tokio::test(flavor = "multi_thread")]
async fn rpc_005_tx_rejection_should_use_standard_json_rpc_error_class() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let http = Client::new();

    for (raw, message_fragment) in [("0x", "empty"), ("0x01", "decode")] {
        let response = rpc_call(
            &http,
            rollup.http_addr,
            "eth_sendRawTransaction",
            json!([raw]),
        )
        .await?;

        assert!(
            response.get("error").is_some(),
            "malformed raw transaction should return JSON-RPC error: {response}"
        );
        assert!(
            response.get("result").is_none() || response["result"].is_null(),
            "eth_sendRawTransaction should not return a result for malformed raw input: {response}"
        );
        let error = rpc_error_object(&response, "eth_sendRawTransaction");
        assert_eq!(
            rpc_error_code(error),
            INVALID_PARAMS_CODE,
            "eth_sendRawTransaction should classify malformed raw input as invalid params: {response}"
        );
        assert!(
            rpc_error_message(error).contains(message_fragment),
            "eth_sendRawTransaction error should mention '{message_fragment}': {response}"
        );
        assert!(
            rpc_error_data_str(error).is_none(),
            "eth_sendRawTransaction should not attach error.data for malformed raw input: {response}"
        );
    }

    let bad_params = rpc_call(
        &http,
        rollup.http_addr,
        "eth_call",
        json!([{}, "latest", "extra-param"]),
    )
    .await?;
    let bad_params_error = rpc_error_object(&bad_params, "eth_call");
    assert_eq!(rpc_error_code(bad_params_error), INVALID_PARAMS_CODE);
    assert!(
        bad_params.get("result").is_none() || bad_params["result"].is_null(),
        "eth_call should not return a result for invalid params: {bad_params}"
    );

    let invalid_tag = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!(["invalid-tag", false]),
    )
    .await?;
    let invalid_tag_error = rpc_error_object(&invalid_tag, "eth_getBlockByNumber");
    assert_eq!(rpc_error_code(invalid_tag_error), INVALID_PARAMS_CODE);
    assert!(
        invalid_tag.get("result").is_none() || invalid_tag["result"].is_null(),
        "eth_getBlockByNumber should not return a result for an invalid tag: {invalid_tag}"
    );

    Ok(())
}

// RPC-006
#[tokio::test(flavor = "multi_thread")]
async fn rpc_006_selector_matrix_accepts_tags_and_eip_1898_objects() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let sender = client.address();
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let set_tx_hash = client.set_value(contract, 7).await;
    let set_receipt = client.wait_for_finalized_receipt(set_tx_hash).await;
    let block_number = set_receipt
        .block_number
        .expect("finalized receipt should include block number");
    let block_hash = set_receipt
        .block_hash
        .expect("finalized receipt should include block hash");
    let number_selector = hex_u64(block_number);
    let number_selector_value = json!(number_selector.clone());
    let hash_selector_value = hash_selector(block_hash);
    let block_number_object = json!({ "blockNumber": number_selector.clone() });

    let code_at = |block: Value| {
        let client = &client;
        async move {
            let code: Bytes = client
                .ws
                .request("eth_getCode", rpc_params![contract, block])
                .await?;
            anyhow::Ok(code)
        }
    };
    let storage_at = |block: Value| {
        let client = &client;
        async move {
            let value: B256 = client
                .ws
                .request("eth_getStorageAt", rpc_params![contract, U256::ZERO, block])
                .await?;
            anyhow::Ok(U256::from_be_slice(value.as_slice()))
        }
    };
    let call_at = |tx: &TransactionRequest, block: Value| {
        let client = &client;
        let mut tx = tx.clone();
        tx.nonce = None;
        async move {
            let output: Bytes = client
                .ws
                .request("eth_call", rpc_params![tx, block])
                .await?;
            anyhow::Ok(U256::from_be_slice(output.as_ref()))
        }
    };
    let estimate_at = |tx: &TransactionRequest, block: Value| {
        let client = &client;
        let mut tx = tx.clone();
        tx.nonce = None;
        async move {
            let estimate: U256 = client
                .ws
                .request("eth_estimateGas", rpc_params![tx, block])
                .await?;
            anyhow::Ok(estimate)
        }
    };

    let by_number: U256 = client
        .ws
        .request(
            "eth_getBalance",
            rpc_params![sender, number_selector.clone()],
        )
        .await?;
    let by_hash: U256 = client
        .ws
        .request(
            "eth_getBalance",
            rpc_params![sender, hash_selector_value.clone()],
        )
        .await?;
    let by_number_object: U256 = client
        .ws
        .request(
            "eth_getBalance",
            rpc_params![sender, block_number_object.clone()],
        )
        .await?;
    assert_eq!(by_number, by_hash);
    assert_eq!(by_number, by_number_object);

    let nonce_by_number = tx_count(&client, sender, number_selector_value.clone()).await?;
    let nonce_by_hash = tx_count(&client, sender, hash_selector_value.clone()).await?;
    let nonce_by_number_object = tx_count(&client, sender, block_number_object.clone()).await?;
    assert_eq!(nonce_by_number, nonce_by_hash);
    assert_eq!(nonce_by_number, nonce_by_number_object);

    let code_by_number = code_at(number_selector_value.clone()).await?;
    let code_by_hash = code_at(hash_selector_value.clone()).await?;
    let code_by_number_object = code_at(block_number_object.clone()).await?;
    assert_eq!(code_by_number, code_by_hash);
    assert_eq!(code_by_number, code_by_number_object);

    let storage_by_number = storage_at(number_selector_value.clone()).await?;
    let storage_by_hash = storage_at(hash_selector_value.clone()).await?;
    let storage_by_number_object = storage_at(block_number_object.clone()).await?;
    assert_eq!(storage_by_number, U256::from(7u64));
    assert_eq!(storage_by_number, storage_by_hash);
    assert_eq!(storage_by_number, storage_by_number_object);
    let storage_word = hex_word_u64(7);
    let http = Client::new();
    let raw_storage_by_number = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getStorageAt",
        json!([format!("{contract:#x}"), "0x0", number_selector.clone()]),
    )
    .await?;
    assert!(
        raw_storage_by_number.get("error").is_none(),
        "eth_getStorageAt(number) should succeed: {raw_storage_by_number}"
    );
    assert_eq!(
        raw_storage_by_number["result"].as_str(),
        Some(storage_word.as_str()),
        "eth_getStorageAt should return a full 32-byte storage word, not a quantity or shortened hex"
    );

    let get_tx = client.make_tx(Some(contract), Some(client.contract.get()));
    let call_by_number = call_at(&get_tx, number_selector_value.clone()).await?;
    let call_by_hash = call_at(&get_tx, hash_selector_value.clone()).await?;
    let call_by_number_object = call_at(&get_tx, block_number_object.clone()).await?;
    assert_eq!(call_by_number, U256::from(7u64));
    assert_eq!(call_by_number, call_by_hash);
    assert_eq!(call_by_number, call_by_number_object);

    let estimate_tx = client.make_tx(Some(contract), Some(client.contract.set(1)));
    let estimate_by_number = estimate_at(&estimate_tx, number_selector_value.clone()).await?;
    let estimate_by_hash = estimate_at(&estimate_tx, hash_selector_value.clone()).await?;
    let estimate_by_number_object = estimate_at(&estimate_tx, block_number_object.clone()).await?;
    assert_eq!(estimate_by_number, estimate_by_hash);
    assert_eq!(estimate_by_number, estimate_by_number_object);

    let block_by_number = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!([number_selector.clone(), false]),
    )
    .await?;
    let block_by_hash_object = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!([hash_selector_value.clone(), false]),
    )
    .await?;
    let block_by_number_object_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!([block_number_object.clone(), false]),
    )
    .await?;
    let block_safe = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!(["safe", false]),
    )
    .await?;
    let block_finalized = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockByNumber",
        json!(["finalized", false]),
    )
    .await?;

    for (name, response) in [
        ("eth_getBlockByNumber(number)", &block_by_number),
        ("eth_getBlockByNumber(blockHash)", &block_by_hash_object),
        (
            "eth_getBlockByNumber(blockNumberObject)",
            &block_by_number_object_response,
        ),
        ("eth_getBlockByNumber(safe)", &block_safe),
        ("eth_getBlockByNumber(finalized)", &block_finalized),
    ] {
        assert!(
            response.get("error").is_none(),
            "{name} should succeed: {response}"
        );
    }
    assert_eq!(
        block_by_number["result"]["hash"], block_by_hash_object["result"]["hash"],
        "blockHash object should resolve to the same block as the numeric selector"
    );
    assert_eq!(
        block_by_number["result"]["hash"], block_by_number_object_response["result"]["hash"],
        "blockNumber object should resolve to the same block as the numeric selector"
    );
    assert_eq!(
        block_safe["result"]["number"], block_finalized["result"]["number"],
        "safe and finalized tags should both be accepted and resolve consistently"
    );

    let receipts_by_number = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockReceipts",
        json!([number_selector]),
    )
    .await?;
    let receipts_by_hash = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockReceipts",
        json!([hash_selector_value]),
    )
    .await?;
    let receipts_by_number_object = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockReceipts",
        json!([block_number_object]),
    )
    .await?;
    for (name, response) in [
        ("eth_getBlockReceipts(number)", &receipts_by_number),
        ("eth_getBlockReceipts(blockHash)", &receipts_by_hash),
        (
            "eth_getBlockReceipts(blockNumberObject)",
            &receipts_by_number_object,
        ),
    ] {
        assert!(
            response.get("error").is_none(),
            "{name} should succeed: {response}"
        );
        assert!(
            response["result"].is_array(),
            "{name} should return a receipts array: {response}"
        );
    }
    assert_eq!(
        receipts_by_number["result"].as_array().unwrap().len(),
        receipts_by_hash["result"].as_array().unwrap().len(),
        "block receipt selector variants should resolve to the same block"
    );
    assert_eq!(
        receipts_by_number["result"].as_array().unwrap().len(),
        receipts_by_number_object["result"]
            .as_array()
            .unwrap()
            .len(),
        "blockNumber object should resolve to the same block receipts"
    );

    Ok(())
}

// RPC-007
#[tokio::test(flavor = "multi_thread")]
async fn rpc_007_web3_client_version_should_be_available() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let http = Client::new();

    let client_version = rpc_call(&http, rollup.http_addr, "web3_clientVersion", json!([])).await?;
    assert!(
        client_version.get("error").is_none(),
        "web3_clientVersion should be implemented: {client_version}"
    );
    assert!(
        client_version["result"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "web3_clientVersion should return a non-empty client identifier: {client_version}"
    );

    let sha3 = rpc_call(&http, rollup.http_addr, "web3_sha3", json!(["0x"])).await?;
    assert!(
        sha3.get("error").is_none(),
        "web3_sha3 should be implemented: {sha3}"
    );
    assert_eq!(
        sha3["result"].as_str(),
        Some("0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"),
        "web3_sha3(0x) should return the canonical Keccak-256 of empty bytes"
    );

    let listening = rpc_call(&http, rollup.http_addr, "net_listening", json!([])).await?;
    assert!(
        listening.get("error").is_none(),
        "net_listening should be implemented: {listening}"
    );
    assert!(
        listening["result"].is_boolean(),
        "net_listening should return a boolean: {listening}"
    );

    let peer_count = rpc_call(&http, rollup.http_addr, "net_peerCount", json!([])).await?;
    assert!(
        peer_count.get("error").is_none(),
        "net_peerCount should be implemented: {peer_count}"
    );
    let peer_count_hex = peer_count["result"]
        .as_str()
        .expect("net_peerCount should return a hex quantity");
    assert!(
        peer_count_hex.starts_with("0x"),
        "net_peerCount should use hex quantity encoding: {peer_count}"
    );
    let _ = parse_hex_u64(peer_count_hex);

    Ok(())
}

// RPC-008
#[tokio::test(flavor = "multi_thread")]
async fn rpc_008_receipt_fee_fields_match_balance_delta() -> anyhow::Result<()> {
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
        "receipt fee/value accounting should exactly match sender balance delta"
    );

    let block_hash = receipt
        .block_hash
        .expect("finalized receipt should include block hash");
    let tx_hash_hex = format!("{tx_hash:#x}");
    let block_hash_hex = format!("{block_hash:#x}");
    let http = Client::new();
    let standalone_receipt = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getTransactionReceipt",
        json!([tx_hash_hex.clone()]),
    )
    .await?;
    let block_receipts = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getBlockReceipts",
        json!([block_hash_hex]),
    )
    .await?;
    assert!(
        standalone_receipt.get("error").is_none(),
        "eth_getTransactionReceipt should succeed: {standalone_receipt}"
    );
    assert!(
        block_receipts.get("error").is_none(),
        "eth_getBlockReceipts should succeed: {block_receipts}"
    );
    let block_receipt = block_receipts["result"]
        .as_array()
        .and_then(|receipts| {
            receipts
                .iter()
                .find(|receipt| receipt["transactionHash"].as_str() == Some(tx_hash_hex.as_str()))
        })
        .expect("eth_getBlockReceipts should include the target receipt");

    for field in [
        "transactionHash",
        "blockHash",
        "blockNumber",
        "gasUsed",
        "effectiveGasPrice",
        "cumulativeGasUsed",
        "status",
    ] {
        assert_eq!(
            standalone_receipt["result"][field], block_receipt[field],
            "{field} should match between eth_getTransactionReceipt and eth_getBlockReceipts"
        );
    }
    assert!(
        standalone_receipt["result"]["gasUsed"].is_string(),
        "receipt should expose gasUsed as a hex quantity string: {standalone_receipt}"
    );
    assert!(
        standalone_receipt["result"]["effectiveGasPrice"].is_string(),
        "receipt should expose effectiveGasPrice as a hex quantity string: {standalone_receipt}"
    );

    Ok(())
}

// RPC-009
#[tokio::test(flavor = "multi_thread")]
async fn rpc_009_pending_trace_matches_original_tx_input() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let provider = alloy_client(rollup.http_addr);
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    rollup.pause_preferred_batches_and_wait().await?;

    let inc_tx_1 = client
        .send_tx(client.make_tx(Some(contract), Some(client.contract.inc())))
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    client.wait_for_receipt(inc_tx_1).await;
    let inc_tx_2 = client
        .send_tx(client.make_tx(Some(contract), Some(client.contract.inc())))
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    client.wait_for_receipt(inc_tx_2).await;

    let pending_value = client
        .query_contract(contract)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    assert_eq!(
        pending_value,
        U256::from(2u64),
        "pending state should already include both increments before tracing"
    );

    let opts = GethDebugTracingOptions::call_tracer(CallConfig::default());
    let trace_1 = provider
        .debug_trace_transaction(inc_tx_1, opts.clone())
        .await?;
    let trace_2 = provider
        .debug_trace_transaction(inc_tx_2, opts.clone())
        .await?;
    let trace_output = |trace: &GethTrace| match trace {
        GethTrace::CallTracer(call_frame) => U256::from_be_slice(
            call_frame
                .output
                .as_ref()
                .expect("callTracer should include output bytes"),
        ),
        other => panic!("expected CallTracer output, got {other:?}"),
    };
    let trace_result_output = |trace_result: &TraceResult| match trace_result {
        TraceResult::Success { result, .. } => trace_output(result),
        TraceResult::Error { error, .. } => panic!("expected successful trace result, got {error}"),
    };

    assert_eq!(
        trace_output(&trace_1),
        U256::from(1u64),
        "tx1 must replay from pre-tx1 state, not current post-mutation state"
    );
    assert_eq!(
        trace_output(&trace_2),
        U256::from(2u64),
        "tx2 must replay from state that includes tx1 but not later mutations"
    );

    let pending_traces = provider
        .debug_trace_block_by_number(BlockNumberOrTag::Pending, opts)
        .await?;
    assert_eq!(
        pending_traces.len(),
        2,
        "pending trace block should contain exactly the two pending increment transactions"
    );
    assert_eq!(
        pending_traces[0].tx_hash(),
        Some(inc_tx_1),
        "first block trace entry should correspond to tx1"
    );
    assert_eq!(
        pending_traces[1].tx_hash(),
        Some(inc_tx_2),
        "second block trace entry should correspond to tx2"
    );
    assert_eq!(
        trace_result_output(&pending_traces[0]),
        U256::from(1u64),
        "block trace for tx1 must replay from pre-tx1 state"
    );
    assert_eq!(
        trace_result_output(&pending_traces[1]),
        U256::from(2u64),
        "block trace for tx2 must replay from post-tx1 / pre-tx2 state"
    );

    Ok(())
}

// RPC-010
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Design-divergence risk test: sync endpoints should not report inclusion before timeout expires under paused batching"]
async fn rpc_010_send_raw_transaction_sync_returns_receipt_under_preferred_sequencer(
) -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches_and_wait().await?;

    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let nonce = tx_count(&client, signer.address(), "latest").await?;
    let chain_id: U64 = client.ws.request("eth_chainId", rpc_params![]).await?;
    let raw = raw_signed_eip1559(
        &signer,
        chain_id.to::<u64>(),
        nonce,
        300_000,
        TxKind::Call(Address::repeat_byte(0x22)),
        U256::from(1),
        Bytes::new(),
        MAX_FEE_PER_GAS,
        MAX_PRIORITY_FEE_PER_GAS,
    )
    .await?;

    let http = Client::new();
    for method in ["eth_sendRawTransactionSync", "realtime_sendRawTransaction"] {
        let response = rpc_call(&http, rollup.http_addr, method, json!([raw, 1])).await?;
        assert!(
            response.get("error").is_some()
                || response.get("result").is_none()
                || response["result"].is_null(),
            "{method} must not report inclusion before a real wait/timeout boundary under paused batching: {response}"
        );
    }

    Ok(())
}

// RPC-011
#[tokio::test(flavor = "multi_thread")]
async fn rpc_011_pruned_log_range_should_not_use_custom_4444_code() -> anyhow::Result<()> {
    let _guard = EnvVarGuard::set("SOV_TEST_CONST_OVERRIDE_EVM_BLOCK_PRUNING_THRESHOLD", "5");
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(2).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let log_tx = client.set_value(contract, 123).await;
    let log_receipt = client.wait_for_finalized_receipt(log_tx).await;
    let log_block = log_receipt
        .block_number
        .expect("log tx should be finalized and have block number");
    let http = Client::new();

    let (get_logs_response, get_logs_with_cursor_response) = poll_until(
        || async {
            rollup.wait_for_rollup_height_advance_by(1).await;
            let get_logs = rpc_call(
                &http,
                rollup.http_addr,
                "eth_getLogs",
                json!([{
                    "fromBlock": hex_u64(log_block),
                    "toBlock": hex_u64(log_block),
                }]),
            )
            .await?;
            let get_logs_with_cursor = rpc_call(
                &http,
                rollup.http_addr,
                "eth_getLogsWithCursor",
                json!([FilterWithCursor {
                    cursor: None,
                    filter: Filter::new()
                        .from_block(BlockNumberOrTag::Number(log_block))
                        .to_block(BlockNumberOrTag::Number(log_block)),
                }]),
            )
            .await?;
            Ok((get_logs, get_logs_with_cursor))
        },
        |(logs, logs_with_cursor)| {
            logs.get("error").is_some() && logs_with_cursor.get("error").is_some()
        },
        "timed out waiting for both log endpoints to enter the pruned-history path",
    )
    .await?;

    for (method, response) in [
        ("eth_getLogs", &get_logs_response),
        ("eth_getLogsWithCursor", &get_logs_with_cursor_response),
    ] {
        let error = rpc_error_object(response, method);
        assert_eq!(
            rpc_error_code(error),
            -32001,
            "{method} should map pruned history to resource-not-found: {response}"
        );
        assert!(
            response.get("result").is_none() || response["result"].is_null(),
            "{method} should not return a result once the range is pruned: {response}"
        );
    }

    Ok(())
}

// RPC-011 (trace path)
#[tokio::test(flavor = "multi_thread")]
async fn rpc_011_debug_trace_pruned_tx_should_not_use_custom_4444_code() -> anyhow::Result<()> {
    let _guard = EnvVarGuard::set("SOV_TEST_CONST_OVERRIDE_EVM_BLOCK_PRUNING_THRESHOLD", "5");
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(2).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    let tx_hash = client.send_eth(Address::ZERO, U256::from(1u64)).await;
    let receipt = client.wait_for_finalized_receipt(tx_hash).await;
    let block_number = receipt
        .block_number
        .expect("finalized receipt should include block number");
    let http = Client::new();

    let (trace_tx_response, trace_block_response) = poll_until(
        || async {
            rollup.wait_for_rollup_height_advance_by(1).await;
            let trace_tx = rpc_call(
                &http,
                rollup.http_addr,
                "debug_traceTransaction",
                json!([tx_hash, {"tracer": "callTracer"}]),
            )
            .await?;
            let trace_block = rpc_call(
                &http,
                rollup.http_addr,
                "debug_traceBlockByNumber",
                json!([hex_u64(block_number), {"tracer": "callTracer"}]),
            )
            .await?;
            Ok((trace_tx, trace_block))
        },
        |(trace_tx, trace_block)| {
            trace_tx.get("error").is_some() && trace_block.get("error").is_some()
        },
        "timed out waiting for both trace endpoints to enter the pruned-history path",
    )
    .await?;

    for (method, response) in [
        ("debug_traceTransaction", &trace_tx_response),
        ("debug_traceBlockByNumber", &trace_block_response),
    ] {
        let error = rpc_error_object(response, method);
        assert_eq!(
            rpc_error_code(error),
            -32001,
            "{method} should map pruned history to resource-not-found: {response}"
        );
        assert!(
            response.get("result").is_none() || response["result"].is_null(),
            "{method} should not return a result once the trace target is pruned: {response}"
        );
    }

    Ok(())
}

// RPC-012
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known compatibility gap: range-based eth_subscribe(logs) should support backfill + live emission"]
async fn rpc_012_log_subscription_should_accept_numeric_block_range() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract = client.alloy_deploy_contract().await;

    let historical_tx = client.alloy_emit_logs(contract, 7, 1).await;
    let historical_receipt = client.wait_for_finalized_receipt(historical_tx).await;
    let historical_block = historical_receipt
        .block_number
        .expect("historical receipt should include block number");

    let ws_client = jsonrpsee::ws_client::WsClientBuilder::default()
        .build(format!("ws://127.0.0.1:{}/rpc", rollup.http_addr.port()))
        .await?;
    let mut subscription = ws_client
        .subscribe::<Value, _>(
            "eth_subscribe",
            rpc_params![
                "logs",
                json!({
                    "address": format!("{contract:#x}"),
                    "fromBlock": hex_u64(historical_block),
                    "toBlock": "latest",
                })
            ],
            "eth_unsubscribe",
        )
        .await?;
    let historical_tx_hex = format!("{historical_tx:#x}");

    let first_event = timeout(SUBSCRIPTION_TIMEOUT, subscription.next())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for historical backfill log"))?
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("subscription closed before historical backfill event"))?;
    assert_eq!(
        first_event["transactionHash"].as_str(),
        Some(historical_tx_hex.as_str()),
        "first subscription event should be the historical backfill log in-range"
    );

    let live_tx = client.alloy_emit_logs(contract, 8, 1).await;
    let live_tx_hex = format!("{live_tx:#x}");
    let second_event = timeout(SUBSCRIPTION_TIMEOUT, subscription.next())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for live log"))?
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("subscription closed before live event"))?;
    assert_eq!(
        second_event["transactionHash"].as_str(),
        Some(live_tx_hex.as_str()),
        "second subscription event should be the live log emitted after subscription setup"
    );
    assert_ne!(
        first_event["transactionHash"], second_event["transactionHash"],
        "backfill and live log events should not be duplicated"
    );
    subscription.unsubscribe().await?;

    Ok(())
}

// RPC-013
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Design divergence sentinel: requires the EVM multi-credential/account-abstraction path to be enabled"]
async fn rpc_013_create_prediction_matches_rpc_nonce_in_demo_harness() -> anyhow::Result<()> {
    anyhow::bail!(
        "enable the multi-credential EVM account-abstraction flow before running this CREATE prediction divergence sentinel"
    );
}

// RPC-014
#[tokio::test(flavor = "multi_thread")]
async fn rpc_014_pending_semantics_matrix_is_internally_consistent() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let sender = client.address();

    let baseline_latest = tx_count(&client, sender, "latest").await?;
    let baseline_pending = tx_count(&client, sender, "pending").await?;
    assert_eq!(
        baseline_latest, baseline_pending,
        "latest and pending should agree before introducing a pending transaction"
    );

    let (finalized_number, finalized_hash) = finalized_block_number_and_hash(&client).await;
    let finalized_selector = hex_u64(finalized_number);

    rollup.pause_preferred_batches_and_wait().await?;
    let tx_hash = client.send_eth(Address::ZERO, U256::from(3u64)).await;
    let pending_receipt = client.wait_for_receipt(tx_hash).await;
    let pending_block_number = pending_receipt
        .block_number
        .expect("pending receipt should include block number");
    let tx_hash_hex = format!("{tx_hash:#x}");
    let pending_block_number_hex = hex_u64(pending_block_number);

    let pending_tx = client
        .ws
        .request::<Value, _>("eth_getTransactionByHash", rpc_params![tx_hash])
        .await?;
    assert_eq!(
        pending_tx["hash"].as_str(),
        Some(tx_hash_hex.as_str()),
        "eth_getTransactionByHash must surface the same pending transaction whose nonce already advanced"
    );
    assert_eq!(
        pending_tx["blockNumber"].as_str(),
        Some(pending_block_number_hex.as_str()),
        "pending transaction lookup must agree with the surfaced pending block number"
    );

    let latest_after_pending = tx_count(&client, sender, "latest").await?;
    let pending_after_pending = tx_count(&client, sender, "pending").await?;
    let finalized_after_pending = tx_count(&client, sender, finalized_selector.clone()).await?;
    let hash_pinned_after_pending =
        tx_count(&client, sender, hash_selector(finalized_hash)).await?;
    assert_eq!(latest_after_pending, baseline_latest + 1);
    assert_eq!(pending_after_pending, latest_after_pending);
    assert_eq!(finalized_after_pending, baseline_latest);
    assert_eq!(hash_pinned_after_pending, baseline_latest);

    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let sealed_tx = client
        .ws
        .request::<Value, _>("eth_getTransactionByHash", rpc_params![tx_hash])
        .await?;
    assert_eq!(
        sealed_tx["hash"].as_str(),
        Some(tx_hash_hex.as_str()),
        "sealed lookup must continue to resolve the original transaction"
    );
    assert_eq!(
        tx_count(&client, sender, "finalized").await?,
        baseline_latest + 1,
        "finalized nonce should advance once the pending transaction seals"
    );

    Ok(())
}

// RPC-015
#[tokio::test(flavor = "multi_thread")]
async fn rpc_015_estimate_gas_is_stable_for_identical_input() -> anyhow::Result<()> {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let contract = deploy_contract_check(&client)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let set_tx = client.make_tx(Some(contract), Some(client.contract.set(0x1234)));
    let emit_logs_tx = client.make_tx(Some(contract), Some(client.contract.emit_logs(7, 3)));
    let revert_tx = client.make_tx(Some(contract), Some(client.contract.always_revert()));

    let set_estimate_1: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&set_tx, "latest"])
        .await?;
    let set_estimate_2: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&set_tx, "latest"])
        .await?;
    let emit_logs_estimate_1: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&emit_logs_tx, "latest"])
        .await?;
    let emit_logs_estimate_2: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&emit_logs_tx, "latest"])
        .await?;

    assert_eq!(set_estimate_1, set_estimate_2);
    assert_eq!(emit_logs_estimate_1, emit_logs_estimate_2);
    assert!(
        set_estimate_1.to::<u64>() > 21_000,
        "set-value estimate should remain above bare transfer cost"
    );
    assert!(
        emit_logs_estimate_1.to::<u64>() > 21_000,
        "event-heavy estimate should remain above bare transfer cost"
    );
    let http = Client::new();
    let revert_estimate = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        json!([serde_json::to_value(&revert_tx)?, "latest"]),
    )
    .await?;
    assert!(
        revert_estimate.get("error").is_some(),
        "reverting estimate must return an error response: {revert_estimate}"
    );
    assert!(
        revert_estimate.get("result").is_none() || revert_estimate["result"].is_null(),
        "eth_estimateGas should not return a result when simulation reverts: {revert_estimate}"
    );
    assert!(
        !rpc_error_message(rpc_error_object(&revert_estimate, "eth_estimateGas")).is_empty(),
        "reverting estimate must include a message: {revert_estimate}"
    );

    Ok(())
}

// RPC-016
#[tokio::test(flavor = "multi_thread")]
async fn rpc_016_eth_send_transaction_should_preserve_user_gas() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let from = signer.address();
    let requested_gas = 0x50_000u64;
    let requested_gas_hex = hex_u64(requested_gas);
    let omitted_request = json!([{
        "from": from,
        "to": Address::ZERO,
        "value": "0x0",
        "maxFeePerGas": hex_u64(MAX_FEE_PER_GAS as u64),
        "maxPriorityFeePerGas": "0x1"
    }]);

    let http = Client::new();
    let estimate_response = rpc_call(
        &http,
        rollup.http_addr,
        "eth_estimateGas",
        omitted_request.clone(),
    )
    .await?;
    assert!(
        estimate_response.get("error").is_none(),
        "eth_estimateGas should succeed for the omitted-gas transfer: {estimate_response}"
    );
    let estimated_gas = rpc_result_hex(&estimate_response);

    let explicit_send = rpc_call(
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
        explicit_send.get("error").is_none(),
        "explicit-gas eth_sendTransaction should succeed: {explicit_send}"
    );
    let explicit_hash = rpc_result_hex(&explicit_send);

    let omitted_send = rpc_call(
        &http,
        rollup.http_addr,
        "eth_sendTransaction",
        omitted_request,
    )
    .await?;
    assert!(
        omitted_send.get("error").is_none(),
        "omitted-gas eth_sendTransaction should succeed: {omitted_send}"
    );
    let omitted_hash = rpc_result_hex(&omitted_send);

    let explicit_tx = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getTransactionByHash",
        json!([explicit_hash]),
    )
    .await?;
    let omitted_tx = rpc_call(
        &http,
        rollup.http_addr,
        "eth_getTransactionByHash",
        json!([omitted_hash]),
    )
    .await?;
    assert!(
        explicit_tx.get("error").is_none(),
        "eth_getTransactionByHash should succeed for explicit-gas tx: {explicit_tx}"
    );
    assert!(
        omitted_tx.get("error").is_none(),
        "eth_getTransactionByHash should succeed for omitted-gas tx: {omitted_tx}"
    );

    assert_eq!(
        explicit_tx["result"]["gas"].as_str(),
        Some(requested_gas_hex.as_str()),
        "explicit gas limit must be preserved exactly"
    );
    assert_eq!(
        omitted_tx["result"]["gas"].as_str(),
        Some(estimated_gas.as_str()),
        "when gas is omitted, eth_sendTransaction must use the node-side estimate"
    );
    assert_ne!(
        explicit_tx["result"]["gas"], omitted_tx["result"]["gas"],
        "explicit and omitted gas paths should remain distinct in the stored transaction"
    );

    Ok(())
}
