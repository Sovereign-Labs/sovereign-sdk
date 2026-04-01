//! E2E tests verifying that block-pinned state reads (by number or hash)
//! do not leak pending transaction effects.
//!
//! When batches are paused, pending transactions are processed but not sealed.
//! Queries pinned to the last sealed block (by number or hash) MUST return
//! pre-pending values, while `latest`/`pending` queries reflect pending state.

use crate::evm::evm_test_helper::{
    deploy_contract_check, hash_selector, hex_u64, set_value_check,
    setup_with_simple_storage_with_ideal_lag, EVM_EXTENSION,
};
use alloy_primitives::{Address, Bytes, B256, U256, U64};
use alloy_rpc_types_eth::TransactionRequest;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde::Serialize;
use serde_json::json;
use sov_demo_rollup::MockDemoRollup;
use sov_eth_client::SimpleStorageClient;
use sov_modules_api::execution_mode::Native;
use sov_test_utils::test_rollup::TestRollup;

async fn nonce_at(client: &SimpleStorageClient, address: Address, block: impl Serialize) -> u64 {
    let count: U64 = client
        .ws
        .request("eth_getTransactionCount", rpc_params![address, block])
        .await
        .unwrap();
    count.to::<u64>()
}

async fn balance_at(client: &SimpleStorageClient, address: Address, block: impl Serialize) -> U256 {
    client
        .ws
        .request("eth_getBalance", rpc_params![address, block])
        .await
        .unwrap()
}

async fn code_at(client: &SimpleStorageClient, address: Address, block: impl Serialize) -> Bytes {
    client
        .ws
        .request("eth_getCode", rpc_params![address, block])
        .await
        .unwrap()
}

async fn storage_at(
    client: &SimpleStorageClient,
    address: Address,
    index: U256,
    block: impl Serialize,
) -> U256 {
    let value: B256 = client
        .ws
        .request("eth_getStorageAt", rpc_params![address, index, block])
        .await
        .unwrap();
    U256::from_be_slice(value.as_slice())
}

async fn eth_call_at(
    client: &SimpleStorageClient,
    tx: &TransactionRequest,
    block: impl Serialize,
) -> Bytes {
    // This suite validates block-pinned state selection, not stale explicit-nonce handling.
    // RPC-003 enforces nonce-too-low for explicit stale nonce in eth_call/estimate paths.
    let mut tx = tx.clone();
    tx.nonce = None;

    client
        .ws
        .request("eth_call", rpc_params![tx, block])
        .await
        .unwrap()
}

async fn estimate_gas_at(
    client: &SimpleStorageClient,
    tx: &TransactionRequest,
    block: impl Serialize,
) -> U256 {
    // Keep estimate requests aligned with current-account nonce semantics by omitting nonce.
    let mut tx = tx.clone();
    tx.nonce = None;

    client
        .ws
        .request("eth_estimateGas", rpc_params![tx, block])
        .await
        .unwrap()
}

async fn estimate_gas_request_at(
    client: &SimpleStorageClient,
    tx: &TransactionRequest,
    block: impl Serialize,
) -> U256 {
    client
        .ws
        .request("eth_estimateGas", rpc_params![tx, block])
        .await
        .unwrap()
}

async fn estimate_gas_request_at_with_state_overrides(
    client: &SimpleStorageClient,
    tx: &TransactionRequest,
    block: impl Serialize,
) -> U256 {
    client
        .ws
        .request("eth_estimateGas", rpc_params![tx, block, json!({})])
        .await
        .unwrap()
}

async fn setup_rollup_and_client() -> (TestRollup<MockDemoRollup<Native>>, SimpleStorageClient) {
    let (rollup, client, _) = setup_with_simple_storage_with_ideal_lag(0, EVM_EXTENSION, 10).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    // produce_enough_finalized_slots() fires off 12 DA blocks via
    // tenderly_produce_blocks but does not wait for them to be fully
    // processed (the wait loop is empty when finalization_blocks == 0).
    // Drain the pipeline so background slot processing cannot advance the
    // finalized head while the test is running.
    rollup.wait_for_node_synced().await.unwrap();
    (rollup, client)
}

async fn sealed_head_number_and_hash(client: &SimpleStorageClient) -> (u64, B256) {
    let head_number = client.block_number().await;
    let head_hash = client
        .eth_get_block_by_number(Some(hex_u64(head_number)))
        .await
        .header
        .hash;
    (head_number, head_hash)
}

/// Block-pinned `eth_getTransactionCount` must not reflect pending nonce changes.
#[tokio::test(flavor = "multi_thread")]
async fn block_pinned_nonce_excludes_pending() {
    let (rollup, client) = setup_rollup_and_client().await;

    let address = client.address();
    let (head_number, head_hash) = sealed_head_number_and_hash(&client).await;
    let sealed_nonce = nonce_at(&client, address, "latest").await;

    rollup
        .pause_preferred_batches_and_wait()
        .await
        .expect("pause should be acknowledged before nonce assertions");
    let tx_hash = client.send_eth(Address::ZERO, U256::from(0x1234)).await;
    client.wait_for_receipt(tx_hash).await;

    assert_eq!(
        nonce_at(&client, address, hex_u64(head_number)).await,
        sealed_nonce,
        "Nonce at block number N must not include pending tx"
    );
    assert_eq!(
        nonce_at(&client, address, hash_selector(head_hash)).await,
        sealed_nonce,
        "Nonce at block hash H must not include pending tx"
    );

    assert_eq!(
        nonce_at(&client, address, "pending").await,
        sealed_nonce + 1,
        "Pending nonce must include the pending tx"
    );
    assert_eq!(
        nonce_at(&client, address, "latest").await,
        sealed_nonce + 1,
        "Latest nonce must include the pending tx"
    );
}

/// Block-pinned `eth_getBalance` must not reflect pending balance changes.
#[tokio::test(flavor = "multi_thread")]
async fn block_pinned_balance_excludes_pending() {
    let (rollup, client) = setup_rollup_and_client().await;

    let receiver = Address::repeat_byte(0xBB);
    let (head_number, head_hash) = sealed_head_number_and_hash(&client).await;

    let sealed_balance = balance_at(&client, receiver, "latest").await;
    assert_eq!(
        sealed_balance,
        U256::ZERO,
        "Receiver should start with zero balance"
    );

    rollup
        .pause_preferred_batches_and_wait()
        .await
        .expect("pause should be acknowledged before balance assertions");
    let transfer_amount = U256::from(0x1_0000_0000u64);
    let tx_hash = client.send_eth(receiver, transfer_amount).await;
    client.wait_for_receipt(tx_hash).await;

    assert_eq!(
        balance_at(&client, receiver, hex_u64(head_number)).await,
        sealed_balance,
        "Balance at block number N must not include pending transfer"
    );
    assert_eq!(
        balance_at(&client, receiver, hash_selector(head_hash)).await,
        sealed_balance,
        "Balance at block hash H must not include pending transfer"
    );

    assert_eq!(
        balance_at(&client, receiver, "pending").await,
        transfer_amount,
        "Pending balance must include the pending transfer"
    );
    assert_eq!(
        balance_at(&client, receiver, "latest").await,
        transfer_amount,
        "Latest balance must include the pending transfer"
    );
}

/// Block-pinned `eth_getCode` must not reflect pending contract deployments.
#[tokio::test(flavor = "multi_thread")]
async fn block_pinned_code_excludes_pending() {
    let (rollup, client) = setup_rollup_and_client().await;

    let (head_number, head_hash) = sealed_head_number_and_hash(&client).await;

    rollup
        .pause_preferred_batches_and_wait()
        .await
        .expect("pause should be acknowledged before code assertions");
    let deploy_tx = client.deploy_contract().await.unwrap();
    let receipt = client.wait_for_receipt(deploy_tx).await;
    let contract_address = receipt.contract_address.unwrap();

    assert!(
        code_at(&client, contract_address, hex_u64(head_number))
            .await
            .is_empty(),
        "Code at block number N must be empty for pending deployment"
    );
    assert!(
        code_at(&client, contract_address, hash_selector(head_hash))
            .await
            .is_empty(),
        "Code at block hash H must be empty for pending deployment"
    );

    assert!(
        !code_at(&client, contract_address, "pending")
            .await
            .is_empty(),
        "Pending must see the deployed contract code"
    );
    assert!(
        !code_at(&client, contract_address, "latest")
            .await
            .is_empty(),
        "Latest must see the deployed contract code"
    );
}

/// Block-pinned `eth_getStorageAt` must not reflect pending storage changes.
#[tokio::test(flavor = "multi_thread")]
async fn block_pinned_storage_excludes_pending() {
    let (rollup, client) = setup_rollup_and_client().await;

    let contract_addr = deploy_contract_check(&client).await.unwrap();
    let initial_value = 0x1234u32;
    set_value_check(&client, contract_addr, initial_value)
        .await
        .unwrap();
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.wait_for_node_synced().await.unwrap();

    let (head_number, head_hash) = sealed_head_number_and_hash(&client).await;

    rollup
        .pause_preferred_batches_and_wait()
        .await
        .expect("pause should be acknowledged before storage assertions");
    let new_value = 0x5678u32;
    let tx_hash = client.set_value(contract_addr, new_value).await;
    client.wait_for_receipt(tx_hash).await;

    assert_eq!(
        storage_at(&client, contract_addr, U256::ZERO, hex_u64(head_number)).await,
        U256::from(initial_value),
        "Storage at block number N must not include pending change"
    );
    assert_eq!(
        storage_at(&client, contract_addr, U256::ZERO, hash_selector(head_hash)).await,
        U256::from(initial_value),
        "Storage at block hash H must not include pending change"
    );

    assert_eq!(
        storage_at(&client, contract_addr, U256::ZERO, "pending").await,
        U256::from(new_value),
        "Pending storage must include the pending change"
    );
    assert_eq!(
        storage_at(&client, contract_addr, U256::ZERO, "latest").await,
        U256::from(new_value),
        "Latest storage must include the pending change"
    );
}

/// Block-pinned `eth_call` must not reflect pending state changes.
#[tokio::test(flavor = "multi_thread")]
async fn block_pinned_eth_call_excludes_pending() {
    let (rollup, client) = setup_rollup_and_client().await;

    let contract_addr = deploy_contract_check(&client).await.unwrap();
    let initial_value = 0x1234u32;
    set_value_check(&client, contract_addr, initial_value)
        .await
        .unwrap();
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.wait_for_node_synced().await.unwrap();

    let (head_number, head_hash) = sealed_head_number_and_hash(&client).await;

    let get_tx = client.make_tx(Some(contract_addr), Some(client.contract.get()));

    rollup
        .pause_preferred_batches_and_wait()
        .await
        .expect("pause should be acknowledged before eth_call assertions");
    let new_value = 0x5678u32;
    let tx_hash = client.set_value(contract_addr, new_value).await;
    client.wait_for_receipt(tx_hash).await;

    assert_eq!(
        U256::from_be_slice(&eth_call_at(&client, &get_tx, hex_u64(head_number)).await),
        U256::from(initial_value),
        "eth_call at block number N must not reflect pending change"
    );
    assert_eq!(
        U256::from_be_slice(&eth_call_at(&client, &get_tx, hash_selector(head_hash)).await),
        U256::from(initial_value),
        "eth_call at block hash H must not reflect pending change"
    );

    assert_eq!(
        U256::from_be_slice(&eth_call_at(&client, &get_tx, "pending").await),
        U256::from(new_value),
        "eth_call at pending must reflect pending change"
    );
    assert_eq!(
        U256::from_be_slice(&eth_call_at(&client, &get_tx, "latest").await),
        U256::from(new_value),
        "eth_call at latest must reflect pending change"
    );
}

/// Block-pinned `eth_estimateGas` must not reflect pending state changes.
#[tokio::test(flavor = "multi_thread")]
async fn block_pinned_estimate_gas_excludes_pending() {
    let (rollup, client) = setup_rollup_and_client().await;

    let contract_addr = deploy_contract_check(&client).await.unwrap();
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.wait_for_node_synced().await.unwrap();

    let (head_number, head_hash) = sealed_head_number_and_hash(&client).await;

    // Baseline at sealed state (slot is still zero): set(non-zero) pays higher SSTORE cost.
    let set_tx = client.make_tx(Some(contract_addr), Some(client.contract.set(0x5678)));

    let baseline_number = estimate_gas_at(&client, &set_tx, hex_u64(head_number)).await;
    let baseline_hash = estimate_gas_at(&client, &set_tx, hash_selector(head_hash)).await;
    assert!(
        baseline_number > U256::ZERO,
        "Baseline gas estimate must be positive"
    );
    assert_eq!(
        baseline_number, baseline_hash,
        "Pinned baseline by number and hash should match"
    );

    rollup
        .pause_preferred_batches_and_wait()
        .await
        .expect("pause should be acknowledged before estimate_gas assertions");
    // Pending mutation makes slot non-zero in current state, lowering cost for the same call.
    let tx_hash = client.set_value(contract_addr, 0x1234).await;
    client.wait_for_receipt(tx_hash).await;

    assert_eq!(
        estimate_gas_at(&client, &set_tx, hex_u64(head_number)).await,
        baseline_number,
        "Gas estimate at block number N must match sealed baseline"
    );
    assert_eq!(
        estimate_gas_at(&client, &set_tx, hash_selector(head_hash)).await,
        baseline_hash,
        "Gas estimate at block hash H must match sealed baseline"
    );

    let pending_estimate = estimate_gas_at(&client, &set_tx, "pending").await;
    assert!(
        pending_estimate > U256::ZERO,
        "Pending gas estimate must be positive"
    );
    assert_ne!(
        pending_estimate, baseline_number,
        "Pending estimate should differ after pending state mutation"
    );
    let latest_estimate = estimate_gas_at(&client, &set_tx, "latest").await;
    assert_eq!(
        latest_estimate, pending_estimate,
        "Latest estimate should match pending estimate in pending-inclusive semantics"
    );
}

/// Block-pinned `eth_estimateGas` must accept the explicit current pending block number
/// on the runtime-parity path.
#[tokio::test(flavor = "multi_thread")]
async fn block_pinned_pending_number_estimate_gas_matches_pending_runtime_parity() {
    let (rollup, client) = setup_rollup_and_client().await;

    let contract_addr = deploy_contract_check(&client).await.unwrap();
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.wait_for_node_synced().await.unwrap();

    let (sealed_head_number, _) = sealed_head_number_and_hash(&client).await;

    let set_tx = client.make_tx(Some(contract_addr), Some(client.contract.set(0x5678)));
    let sealed_estimate = estimate_gas_at(&client, &set_tx, hex_u64(sealed_head_number)).await;

    rollup
        .pause_preferred_batches_and_wait()
        .await
        .expect("pause should be acknowledged before estimate_gas assertions");
    let tx_hash = client.set_value(contract_addr, 0x1234).await;
    client.wait_for_receipt(tx_hash).await;

    let pending_block_number = client.block_number().await;
    let mut explicit_request = set_tx.clone();
    explicit_request.nonce = Some(nonce_at(&client, client.address(), "pending").await);

    let pending_by_number =
        estimate_gas_request_at(&client, &explicit_request, hex_u64(pending_block_number)).await;
    let pending_by_tag = estimate_gas_request_at(&client, &explicit_request, "pending").await;

    assert_eq!(
        pending_by_number, pending_by_tag,
        "Explicit current pending block number must match the pending estimate"
    );
    assert_ne!(
        pending_by_number, sealed_estimate,
        "Explicit current pending block number must not fall back to the last sealed state"
    );
}

/// Block-pinned `eth_estimateGas` must accept the explicit current pending block number
/// on the legacy estimator path when overrides are present.
#[tokio::test(flavor = "multi_thread")]
async fn block_pinned_pending_number_estimate_gas_matches_pending_with_state_overrides() {
    let (rollup, client) = setup_rollup_and_client().await;

    let contract_addr = deploy_contract_check(&client).await.unwrap();
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.wait_for_node_synced().await.unwrap();

    let (sealed_head_number, _) = sealed_head_number_and_hash(&client).await;

    let set_tx = client.make_tx(Some(contract_addr), Some(client.contract.set(0x5678)));
    let sealed_estimate = estimate_gas_at(&client, &set_tx, hex_u64(sealed_head_number)).await;

    rollup
        .pause_preferred_batches_and_wait()
        .await
        .expect("pause should be acknowledged before estimate_gas assertions");
    let tx_hash = client.set_value(contract_addr, 0x1234).await;
    client.wait_for_receipt(tx_hash).await;

    let pending_block_number = client.block_number().await;
    let mut explicit_request = set_tx.clone();
    explicit_request.nonce = Some(nonce_at(&client, client.address(), "pending").await);

    let pending_by_number = estimate_gas_request_at_with_state_overrides(
        &client,
        &explicit_request,
        hex_u64(pending_block_number),
    )
    .await;
    let pending_by_tag =
        estimate_gas_request_at_with_state_overrides(&client, &explicit_request, "pending").await;

    assert_eq!(
        pending_by_number, pending_by_tag,
        "Explicit current pending block number must match the pending estimate with overrides"
    );
    assert_ne!(
        pending_by_number, sealed_estimate,
        "Explicit current pending block number with overrides must not fall back to the last sealed state"
    );
}
