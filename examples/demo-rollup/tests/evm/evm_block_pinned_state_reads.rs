//! E2E tests verifying that block-pinned state reads (by number or hash)
//! do not leak pending transaction effects.
//!
//! When batches are paused, pending transactions are processed but not sealed.
//! Queries pinned to the last sealed block (by number or hash) MUST return
//! pre-pending values, while `latest`/`pending` queries reflect pending state.

use crate::evm::evm_test_helper::{
    deploy_contract_check, set_value_check, setup_with_simple_storage, EVM_EXTENSION,
};
use alloy_primitives::{Address, Bytes, B256, U256, U64};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde::Serialize;
use serde_json::json;
use sov_eth_client::SimpleStorageClient;

// =========================================================================
// Helpers: block-pinned JSON-RPC queries
// =========================================================================

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
    tx: impl Serialize,
    block: impl Serialize,
) -> Bytes {
    client
        .ws
        .request("eth_call", rpc_params![tx, block])
        .await
        .unwrap()
}

async fn estimate_gas_at(
    client: &SimpleStorageClient,
    tx: impl Serialize,
    block: impl Serialize,
) -> U256 {
    client
        .ws
        .request("eth_estimateGas", rpc_params![tx, block])
        .await
        .unwrap()
}

fn number_selector(n: u64) -> String {
    format!("0x{n:x}")
}

fn hash_selector(hash: B256) -> serde_json::Value {
    json!({
        "blockHash": format!("{:#x}", hash),
        "requireCanonical": true
    })
}

// =========================================================================
// Tests
// =========================================================================

/// Block-pinned `eth_getTransactionCount` must not reflect pending nonce changes.
#[tokio::test(flavor = "multi_thread")]
async fn block_pinned_nonce_excludes_pending() {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let address = client.address();
    let head_number = client.block_number().await;
    let head_block = client
        .eth_get_block_by_number(Some(number_selector(head_number)))
        .await;
    let head_hash = head_block.header.hash;
    let sealed_nonce = nonce_at(&client, address, "latest").await;

    rollup.pause_preferred_batches().await;
    let _tx = client.send_eth(Address::ZERO, U256::from(0x1234)).await;

    assert_eq!(
        client.block_number().await,
        head_number,
        "Block number must not advance while batches are paused"
    );

    assert_eq!(
        nonce_at(&client, address, number_selector(head_number)).await,
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
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let receiver = Address::repeat_byte(0xBB);
    let head_number = client.block_number().await;
    let head_block = client
        .eth_get_block_by_number(Some(number_selector(head_number)))
        .await;
    let head_hash = head_block.header.hash;

    let sealed_balance = balance_at(&client, receiver, "latest").await;
    assert_eq!(
        sealed_balance,
        U256::ZERO,
        "Receiver should start with zero balance"
    );

    rollup.pause_preferred_batches().await;
    let transfer_amount = U256::from(0x1_0000_0000u64);
    let _tx = client.send_eth(receiver, transfer_amount).await;
    assert_eq!(
        client.block_number().await,
        head_number,
        "Block number must not advance while batches are paused"
    );

    assert_eq!(
        balance_at(&client, receiver, number_selector(head_number)).await,
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
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let head_number = client.block_number().await;
    let head_block = client
        .eth_get_block_by_number(Some(number_selector(head_number)))
        .await;
    let head_hash = head_block.header.hash;

    rollup.pause_preferred_batches().await;
    let deploy_tx = client.deploy_contract().await.unwrap();
    let receipt = client.wait_for_receipt(deploy_tx).await;
    let contract_address = receipt.contract_address.unwrap();
    assert_eq!(
        client.block_number().await,
        head_number,
        "Block number must not advance while batches are paused"
    );

    assert!(
        code_at(&client, contract_address, number_selector(head_number))
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
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let contract_addr = deploy_contract_check(&client).await.unwrap();
    let initial_value = 0x1234u32;
    set_value_check(&client, contract_addr, initial_value)
        .await
        .unwrap();
    rollup.wait_for_next_blocks(1).await;

    let head_number = client.block_number().await;
    let head_block = client
        .eth_get_block_by_number(Some(number_selector(head_number)))
        .await;
    let head_hash = head_block.header.hash;

    rollup.pause_preferred_batches().await;
    let new_value = 0x5678u32;
    let _tx = client.set_value(contract_addr, new_value).await;
    assert_eq!(
        client.block_number().await,
        head_number,
        "Block number must not advance while batches are paused"
    );

    assert_eq!(
        storage_at(
            &client,
            contract_addr,
            U256::ZERO,
            number_selector(head_number)
        )
        .await,
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
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let contract_addr = deploy_contract_check(&client).await.unwrap();
    let initial_value = 0x1234u32;
    set_value_check(&client, contract_addr, initial_value)
        .await
        .unwrap();
    rollup.wait_for_next_blocks(1).await;

    let head_number = client.block_number().await;
    let head_block = client
        .eth_get_block_by_number(Some(number_selector(head_number)))
        .await;
    let head_hash = head_block.header.hash;

    let get_tx = client.make_tx(Some(contract_addr), Some(client.contract.get()));

    rollup.pause_preferred_batches().await;
    let new_value = 0x5678u32;
    let _tx = client.set_value(contract_addr, new_value).await;
    assert_eq!(
        client.block_number().await,
        head_number,
        "Block number must not advance while batches are paused"
    );

    assert_eq!(
        U256::from_be_slice(&eth_call_at(&client, &get_tx, number_selector(head_number)).await),
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
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let contract_addr = deploy_contract_check(&client).await.unwrap();
    rollup.wait_for_next_blocks(1).await;

    let head_number = client.block_number().await;
    let head_block = client
        .eth_get_block_by_number(Some(number_selector(head_number)))
        .await;
    let head_hash = head_block.header.hash;

    // Baseline at sealed state (slot is still zero): set(non-zero) pays higher SSTORE cost.
    let set_tx = client.make_tx(Some(contract_addr), Some(client.contract.set(0x5678)));

    let baseline = estimate_gas_at(&client, &set_tx, "latest").await;
    assert!(
        baseline > U256::ZERO,
        "Baseline gas estimate must be positive"
    );

    rollup.pause_preferred_batches().await;
    // Pending mutation makes slot non-zero in current state, lowering cost for the same call.
    let _tx = client.set_value(contract_addr, 0x1234).await;
    assert_eq!(
        client.block_number().await,
        head_number,
        "Block number must not advance while batches are paused"
    );

    assert_eq!(
        estimate_gas_at(&client, &set_tx, number_selector(head_number)).await,
        baseline,
        "Gas estimate at block number N must match sealed baseline"
    );
    assert_eq!(
        estimate_gas_at(&client, &set_tx, hash_selector(head_hash)).await,
        baseline,
        "Gas estimate at block hash H must match sealed baseline"
    );

    let pending_estimate = estimate_gas_at(&client, &set_tx, "pending").await;
    assert!(
        pending_estimate > U256::ZERO,
        "Pending gas estimate must be positive"
    );
    assert_ne!(
        pending_estimate, baseline,
        "Pending estimate should differ after pending state mutation"
    );
    let latest_estimate = estimate_gas_at(&client, &set_tx, "latest").await;
    assert_eq!(
        latest_estimate, pending_estimate,
        "Latest estimate should match pending estimate in pending-inclusive semantics"
    );
}
