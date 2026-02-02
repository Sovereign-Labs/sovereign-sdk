use crate::evm::evm_test_helper::{
    create_simple_storage_client, setup_with_simple_storage, EVM_EXTENSION,
    SECONDARY_SENDER_PRIV_KEY,
};
use alloy_primitives::{Address, B256, U256, U64};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde::Serialize;
use serde_json::json;
use sov_demo_rollup::MockDemoRollup;
use sov_eth_client::SimpleStorageClient;
use sov_modules_api::execution_mode::Native;
use sov_test_utils::test_rollup::TestRollup;

async fn setup_rollup() -> (TestRollup<MockDemoRollup<Native>>, SimpleStorageClient) {
    let (rollup, client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    (rollup, client)
}

async fn nonce_at_tag(client: &SimpleStorageClient, address: Address, tag: &str) -> u64 {
    get_tx_count(client, address, tag).await
}

async fn nonce_at_number(client: &SimpleStorageClient, address: Address, number: u64) -> u64 {
    get_tx_count(client, address, format!("0x{number:x}")).await
}

async fn nonce_at_hash(
    client: &SimpleStorageClient,
    address: Address,
    hash: B256,
    require_canonical: bool,
) -> u64 {
    let selector = json!({
        "blockHash": format!("{:#x}", hash),
        "requireCanonical": require_canonical
    });
    get_tx_count(client, address, selector).await
}

async fn assert_equal_nonces_for_tags(
    client: &SimpleStorageClient,
    address: Address,
    tags: &[&str],
    context: &str,
) {
    let mut previous: Option<(&str, u64)> = None;
    for tag in tags {
        let nonce = nonce_at_tag(client, address, tag).await;
        if let Some((prev_tag, prev_nonce)) = previous {
            assert_eq!(nonce, prev_nonce, "{context}: {prev_tag} != {tag}");
        }
        previous = Some((tag, nonce));
    }
}

// ===========================================================================
// Helper Functions
// ===========================================================================

async fn get_tx_count<P: Serialize>(
    client: &SimpleStorageClient,
    address: Address,
    block: P,
) -> u64 {
    let count: U64 = client
        .ws
        .request("eth_getTransactionCount", rpc_params![address, block])
        .await
        .unwrap();
    count.to::<u64>()
}

async fn try_get_tx_count<P: Serialize>(
    client: &SimpleStorageClient,
    address: Address,
    block: P,
) -> Result<u64, jsonrpsee::core::ClientError> {
    let count: U64 = client
        .ws
        .request("eth_getTransactionCount", rpc_params![address, block])
        .await?;
    Ok(count.to::<u64>())
}

// ===========================================================================
// Block Selector Tests
// ===========================================================================

/// TC01: Verify that omitting the block selector defaults to "latest".
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_default_selector_equals_latest() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    let address = client.address();

    // Call with explicit "latest"
    let latest_nonce = nonce_at_tag(&client, address, "latest").await;

    // Call with no block selector (None serializes to null in JSON)
    let default_nonce_u64: U64 = client
        .ws
        .request("eth_getTransactionCount", rpc_params![address])
        .await
        .unwrap();
    let default_nonce = default_nonce_u64.to::<u64>();

    assert_eq!(
        default_nonce, latest_nonce,
        "Default block selector should equal 'latest'"
    );

    Ok(())
}

/// TC03: Verify "earliest" returns the genesis nonce (0 for fresh addresses).
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_earliest_returns_genesis_nonce() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    // Use a deterministic address that never had transactions
    let fresh_address = Address::repeat_byte(0x11);

    let earliest_nonce = nonce_at_tag(&client, fresh_address, "earliest").await;

    assert_eq!(
        earliest_nonce, 0,
        "Earliest nonce for fresh address should be 0"
    );

    Ok(())
}

/// TC04: Verify "safe" and "finalized" return consistent values.
/// Note: safe/finalized refer to sealed blocks, while latest may include pending transactions.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_safe_finalized_semantics() -> anyhow::Result<()> {
    let (rollup, client) = setup_rollup().await;

    let address = client.address();

    // Send a transaction and wait for it to be sealed (finalized)
    let tx_hash = client.send_eth(Address::ZERO, U256::from(0x1234)).await;
    client.wait_for_finalized_receipt(tx_hash).await;

    // Wait for the next block to ensure finalization
    rollup.wait_for_next_blocks(1).await;

    assert_equal_nonces_for_tags(
        &client,
        address,
        &["safe", "finalized", "latest"],
        "After finalization, selector nonces should match",
    )
    .await;

    Ok(())
}

/// TC02: Verify latest vs pending behavior (in our rollup, they are equal because
/// latest includes pending transactions).
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_latest_vs_pending() -> anyhow::Result<()> {
    let (rollup, client) = setup_rollup().await;
    rollup.pause_preferred_batches().await;

    let address = client.address();
    let head_number = client.block_number().await;

    assert_equal_nonces_for_tags(
        &client,
        address,
        &["latest", "pending"],
        "Before tx: latest and pending should be equal",
    )
    .await;
    let latest_nonce = nonce_at_tag(&client, address, "latest").await;

    // Send a transaction (won't be sealed because batches are paused)
    let _tx_hash = client.send_eth(Address::ZERO, U256::from(0x5678)).await;
    let head_number_after = client.block_number().await;
    assert_eq!(
        head_number_after, head_number,
        "Block number should not change while batches are paused"
    );

    // In our rollup design, both latest and pending reflect the pending tx
    assert_equal_nonces_for_tags(
        &client,
        address,
        &["latest", "pending"],
        "In our rollup, latest == pending (both include pending transactions)",
    )
    .await;
    let latest_after = nonce_at_tag(&client, address, "latest").await;
    assert_eq!(
        latest_after,
        latest_nonce + 1,
        "Nonce should increment after sending transaction"
    );

    Ok(())
}

// ===========================================================================
// Block Hash/Number Tests
// ===========================================================================

/// TC05-08: Verify transaction count queries by block number and block hash.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_block_number_and_hash() -> anyhow::Result<()> {
    let (rollup, client) = setup_rollup().await;
    rollup.pause_preferred_batches().await;

    let head_number = client.block_number().await;
    let head_block = client
        .eth_get_block_by_number(Some(format!("0x{head_number:x}")))
        .await;
    let head_hash = head_block.header.hash;
    assert_ne!(head_hash, B256::ZERO);

    let address = client.address();
    let head_nonce = nonce_at_number(&client, address, head_number).await;

    let _tx_hash = client.send_eth(Address::ZERO, U256::from(0x9ABC)).await;
    let head_number_after = client.block_number().await;
    assert_eq!(head_number_after, head_number);

    // Nonce at historical block should remain unchanged
    let head_nonce_after = nonce_at_number(&client, address, head_number).await;
    assert_eq!(
        head_nonce_after, head_nonce,
        "Historical block nonce should not change"
    );

    // Query by block hash with requireCanonical: true
    let by_hash_true_nonce = nonce_at_hash(&client, address, head_hash, true).await;
    assert_eq!(
        by_hash_true_nonce, head_nonce,
        "Block hash query (canonical=true) should match block number query"
    );

    // Query by block hash with requireCanonical: false
    let by_hash_false_nonce = nonce_at_hash(&client, address, head_hash, false).await;
    assert_eq!(
        by_hash_false_nonce, head_nonce,
        "Block hash query (canonical=false) should match block number query"
    );

    Ok(())
}

/// TC09: Verify that querying with an unknown block hash returns an error.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_unknown_blockhash_errors() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    let address = client.address();

    // Use a random hash that doesn't exist
    let unknown_hash = B256::from([0x42; 32]);
    let selector = json!({
        "blockHash": format!("{:#x}", unknown_hash),
        "requireCanonical": true
    });
    let result = try_get_tx_count(&client, address, selector).await;
    assert!(
        result.is_err(),
        "Query with unknown block hash should return error"
    );

    Ok(())
}

/// TC17: Verify that querying a future block number returns an error.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_future_block_errors() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    let address = client.address();
    let current_block = client.block_number().await;

    // Request nonce at a block far in the future
    let future_block = current_block + 1000;
    let result = try_get_tx_count(&client, address, format!("0x{future_block:x}")).await;

    assert!(
        result.is_err(),
        "Query with future block number should return error"
    );

    Ok(())
}

// ===========================================================================
// Nonce Increment Tests
// ===========================================================================

/// TC10: Verify that a fresh (never-used) address has nonce 0.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_initial_nonce_is_zero() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    // Generate a deterministic address that has never transacted
    let fresh_address = Address::repeat_byte(0x11);

    let nonce = nonce_at_tag(&client, fresh_address, "latest").await;

    assert_eq!(nonce, 0, "Fresh address should have nonce 0");

    Ok(())
}

/// TC18: Verify that a non-existent address returns nonce 0.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_non_existent_address_returns_zero() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    // Use a deterministic but unlikely-to-exist address
    let non_existent = Address::repeat_byte(0xDE);

    let nonce = nonce_at_tag(&client, non_existent, "latest").await;

    assert_eq!(nonce, 0, "Non-existent address should return nonce 0");

    Ok(())
}

/// TC11: Verify that nonce increments by exactly 1 after a successful transaction.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_increments_by_one() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    let address = client.address();
    let nonce_before = nonce_at_tag(&client, address, "latest").await;

    let tx_hash = client.send_eth(Address::ZERO, U256::from(0xDEAD)).await;
    client.wait_for_receipt(tx_hash).await;

    let nonce_after = nonce_at_tag(&client, address, "latest").await;

    assert_eq!(
        nonce_after,
        nonce_before + 1,
        "Nonce should increment by exactly 1 after successful transaction"
    );

    Ok(())
}

/// TC12: Verify behavior when a transaction would revert.
/// Note: This rollup pre-simulates transactions and rejects ones that would revert.
/// Unlike mainnet Ethereum where reverted transactions are published and consume nonces,
/// this rollup rejects them at submission time, so the nonce is NOT consumed.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_reverted_tx_does_not_consume_nonce() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    // Deploy the SimpleStorage contract first
    let deploy_tx_hash = client.deploy_contract().await.unwrap();
    let deploy_receipt = client.wait_for_receipt(deploy_tx_hash).await;
    let contract_address = deploy_receipt.contract_address.unwrap();

    let address = client.address();
    let nonce_before = nonce_at_tag(&client, address, "latest").await;

    // Call alwaysRevert() which will be rejected at submission time (pre-simulation)
    let result = client.always_reverts(contract_address).await;

    // The transaction should be rejected
    assert!(
        result.is_err(),
        "Reverting transaction should be rejected at submission time"
    );
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("Revert"),
        "Error should indicate a revert: {err_str}"
    );

    let nonce_after = nonce_at_tag(&client, address, "latest").await;

    // In this rollup, rejected transactions do NOT consume nonces
    assert_eq!(
        nonce_after, nonce_before,
        "Rejected (pre-simulated revert) transaction should NOT consume nonce"
    );

    Ok(())
}

/// TC13: Verify sequential transactions increment nonce monotonically.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_sequential_monotonicity() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    let address = client.address();
    let initial_nonce = nonce_at_tag(&client, address, "latest").await;

    // Send 3 sequential transactions
    for i in 0..3 {
        let tx_hash = client.send_eth(Address::ZERO, U256::from(0x100 + i)).await;
        client.wait_for_receipt(tx_hash).await;

        let current_nonce = nonce_at_tag(&client, address, "latest").await;
        assert_eq!(
            current_nonce,
            initial_nonce + i + 1,
            "After transaction {}, nonce should be {}",
            i + 1,
            initial_nonce + i + 1
        );
    }

    Ok(())
}

/// TC14: Verify per-address nonce isolation with interleaved transactions.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_per_address_isolation() -> anyhow::Result<()> {
    let (rollup, client_a) = setup_rollup().await;

    // Create a second client with a different private key
    let client_b = create_simple_storage_client(rollup.http_addr, SECONDARY_SENDER_PRIV_KEY).await;

    let address_a = client_a.address();
    let address_b = client_b.address();
    assert_ne!(
        address_a, address_b,
        "Clients should have different addresses"
    );

    let nonce_a_initial = nonce_at_tag(&client_a, address_a, "latest").await;
    let nonce_b_initial = nonce_at_tag(&client_a, address_b, "latest").await;

    // Interleaved transactions: A1, B1, B2, A2
    let tx_a1 = client_a.send_eth(Address::ZERO, U256::from(0xA1)).await;
    client_a.wait_for_receipt(tx_a1).await;

    let tx_b1 = client_b.send_eth(Address::ZERO, U256::from(0xB1)).await;
    client_b.wait_for_receipt(tx_b1).await;

    let tx_b2 = client_b.send_eth(Address::ZERO, U256::from(0xB2)).await;
    client_b.wait_for_receipt(tx_b2).await;

    let tx_a2 = client_a.send_eth(Address::ZERO, U256::from(0xA2)).await;
    client_a.wait_for_receipt(tx_a2).await;

    let nonce_a_final = nonce_at_tag(&client_a, address_a, "latest").await;
    let nonce_b_final = nonce_at_tag(&client_a, address_b, "latest").await;

    assert_eq!(
        nonce_a_final,
        nonce_a_initial + 2,
        "Address A should have nonce incremented by 2"
    );
    assert_eq!(
        nonce_b_final,
        nonce_b_initial + 2,
        "Address B should have nonce incremented by 2"
    );

    Ok(())
}

// ===========================================================================
// Cross-Validation Tests
// ===========================================================================

/// TC15: Verify pending block nonce consistency.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_pending_block_nonce_consistency() -> anyhow::Result<()> {
    let (rollup, client) = setup_rollup().await;
    rollup.pause_preferred_batches().await;

    let address = client.address();

    // Get nonce at the sealed block
    let sealed_nonce = nonce_at_tag(&client, address, "latest").await;

    // Send multiple transactions while batches are paused
    let pending_tx_count = 3u64;
    for i in 0..pending_tx_count {
        let _tx_hash = client.send_eth(Address::ZERO, U256::from(0x200 + i)).await;
    }

    // Pending nonce should equal sealed_nonce + pending_tx_count
    let pending_nonce = nonce_at_tag(&client, address, "pending").await;

    assert_eq!(
        pending_nonce,
        sealed_nonce + pending_tx_count,
        "Pending nonce should equal sealed nonce plus pending transaction count"
    );

    Ok(())
}

/// TC16: Verify that nonce delta between blocks equals the number of receipts.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_sealed_receipts_nonce_delta() -> anyhow::Result<()> {
    let (rollup, client) = setup_rollup().await;

    let address = client.address();

    // Get nonce at block H0
    let h0_number = client.block_number().await;
    let nonce_h0 = nonce_at_number(&client, address, h0_number).await;

    // Send transactions and wait for them to be sealed
    let tx_count = 2u64;
    for i in 0..tx_count {
        let tx_hash = client.send_eth(Address::ZERO, U256::from(0x300 + i)).await;
        client.wait_for_finalized_receipt(tx_hash).await;
    }

    // Wait for a new block
    rollup.wait_for_next_blocks(1).await;

    // Get nonce at block H1
    let h1_number = client.block_number().await;
    assert!(
        h1_number > h0_number,
        "A new block should have been produced"
    );

    let nonce_h1 = nonce_at_number(&client, address, h1_number).await;

    assert_eq!(
        nonce_h1,
        nonce_h0 + tx_count,
        "Nonce delta between blocks should equal the number of transactions sent"
    );

    Ok(())
}

// ===========================================================================
// DIVERGENCE TESTS - Verify different selectors return DIFFERENT values
// ===========================================================================

/// Verify "earliest" returns genesis nonce, which MUST DIFFER from "latest" after transactions.
/// This catches bugs where "earliest" incorrectly returns current state.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_earliest_diverges_from_latest() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    let address = client.address();

    // Send a transaction so sender has nonce > 0
    let tx_hash = client.send_eth(Address::ZERO, U256::from(0x4321)).await;
    client.wait_for_receipt(tx_hash).await;

    let earliest_nonce = nonce_at_tag(&client, address, "earliest").await;
    let latest_nonce = nonce_at_tag(&client, address, "latest").await;

    // The sender started with nonce 0 at genesis
    assert_eq!(
        earliest_nonce, 0,
        "Sender's nonce at 'earliest' (genesis) should be 0"
    );

    // After transaction, latest should be > 0
    assert!(
        latest_nonce > 0,
        "Sender's nonce at 'latest' should be > 0 after transaction"
    );

    // KEY: earliest and latest MUST be different
    assert_ne!(
        earliest_nonce, latest_nonce,
        "BUG: 'earliest' and 'latest' return same value - earliest should be genesis state"
    );

    Ok(())
}

/// Verify historical block query returns state AT that block, not current state.
/// This catches bugs where historical queries return current/latest state.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_historical_block_diverges_from_current() -> anyhow::Result<()> {
    let (rollup, client) = setup_rollup().await;

    let address = client.address();

    // Record state at block H0
    let h0_number = client.block_number().await;
    let nonce_at_h0_before = nonce_at_number(&client, address, h0_number).await;

    // Send transaction and wait for finalization in a new block
    let tx_hash = client.send_eth(Address::ZERO, U256::from(0x5432)).await;
    client.wait_for_finalized_receipt(tx_hash).await;
    rollup.wait_for_next_blocks(1).await;

    let h1_number = client.block_number().await;
    assert!(h1_number > h0_number, "New block should be produced");

    // Query nonce at old block H0 - should still be the old value
    let nonce_at_h0_after = nonce_at_number(&client, address, h0_number).await;

    // Query nonce at new block H1 - should be incremented
    let nonce_at_h1 = nonce_at_number(&client, address, h1_number).await;

    assert_eq!(
        nonce_at_h0_after, nonce_at_h0_before,
        "Nonce at historical block H0 should not change"
    );

    assert_eq!(
        nonce_at_h1,
        nonce_at_h0_before + 1,
        "Nonce at H1 should be H0 nonce + 1"
    );

    // KEY: Historical and current MUST be different
    assert_ne!(
        nonce_at_h0_after, nonce_at_h1,
        "BUG: nonce(H0) == nonce(H1) - historical query returns current state"
    );

    Ok(())
}

/// Verify block hash queries return correct historical values that DIFFER across blocks.
/// This catches bugs where block hash queries return current state.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_block_hash_returns_correct_historical() -> anyhow::Result<()> {
    let (rollup, client) = setup_rollup().await;

    let address = client.address();

    // Get block H0 info
    let h0_number = client.block_number().await;
    let h0_block = client
        .eth_get_block_by_number(Some(format!("0x{h0_number:x}")))
        .await;
    let h0_hash = h0_block.header.hash;
    let nonce_at_h0 = nonce_at_hash(&client, address, h0_hash, true).await;

    // Send transaction and finalize
    let tx_hash = client.send_eth(Address::ZERO, U256::from(0x6543)).await;
    client.wait_for_finalized_receipt(tx_hash).await;
    rollup.wait_for_next_blocks(1).await;

    // Get block H1 info
    let h1_number = client.block_number().await;
    assert!(h1_number > h0_number, "New block should be produced");
    let h1_block = client
        .eth_get_block_by_number(Some(format!("0x{h1_number:x}")))
        .await;
    let h1_hash = h1_block.header.hash;

    // Query by both hashes
    let nonce_by_h0_hash = nonce_at_hash(&client, address, h0_hash, true).await;
    let nonce_by_h1_hash = nonce_at_hash(&client, address, h1_hash, true).await;

    assert_eq!(
        nonce_by_h0_hash, nonce_at_h0,
        "Nonce by H0 hash should match original H0 nonce"
    );

    assert_eq!(
        nonce_by_h1_hash,
        nonce_at_h0 + 1,
        "Nonce by H1 hash should be H0 nonce + 1"
    );

    // KEY: Hash queries for different blocks MUST return different values
    assert_ne!(
        nonce_by_h0_hash, nonce_by_h1_hash,
        "BUG: nonce(hash_H0) == nonce(hash_H1) - block hash query returns current state"
    );

    Ok(())
}

/// Verify "safe" and "finalized" exclude pending transactions (unlike "pending"/"latest").
/// This catches bugs where safe/finalized incorrectly include pending state.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_safe_finalized_exclude_pending() -> anyhow::Result<()> {
    let (rollup, client) = setup_rollup().await;
    rollup.pause_preferred_batches().await;

    let address = client.address();

    // Record sealed state
    let sealed_block = client.block_number().await;
    let nonce_at_sealed_block = nonce_at_number(&client, address, sealed_block).await;

    // Send pending transaction (batches paused, won't be sealed)
    let _tx_hash = client.send_eth(Address::ZERO, U256::from(0x7654)).await;

    // Verify block number hasn't changed
    let current_block = client.block_number().await;
    assert_eq!(
        current_block, sealed_block,
        "Block should not advance while batches paused"
    );

    // Query all selectors
    let nonce_safe = nonce_at_tag(&client, address, "safe").await;
    let nonce_finalized = nonce_at_tag(&client, address, "finalized").await;
    let nonce_pending = nonce_at_tag(&client, address, "pending").await;

    // safe and finalized should return sealed state (before pending tx)
    assert_eq!(
        nonce_safe, nonce_at_sealed_block,
        "'safe' should return sealed block nonce"
    );
    assert_eq!(
        nonce_finalized, nonce_at_sealed_block,
        "'finalized' should return sealed block nonce"
    );

    // pending should include the pending tx
    assert_eq!(
        nonce_pending,
        nonce_at_sealed_block + 1,
        "'pending' should include pending transaction"
    );

    // KEY: safe/finalized MUST differ from pending when there are pending txs
    assert_ne!(
        nonce_safe, nonce_pending,
        "BUG: 'safe' == 'pending' - safe includes pending transactions"
    );
    assert_ne!(
        nonce_finalized, nonce_pending,
        "BUG: 'finalized' == 'pending' - finalized includes pending transactions"
    );

    Ok(())
}

/// Verify querying at block 0 (genesis) returns nonce 0 for sender.
/// This catches bugs where genesis block query is broken.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_genesis_block_nonce() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    let address = client.address();

    // Send a transaction so current nonce > 0
    let tx_hash = client.send_eth(Address::ZERO, U256::from(0x8765)).await;
    client.wait_for_receipt(tx_hash).await;

    // Query at block 0 (genesis)
    let nonce_at_genesis = nonce_at_number(&client, address, 0).await;

    // Query at latest
    let nonce_at_latest = nonce_at_tag(&client, address, "latest").await;

    // At genesis, sender should have nonce 0
    assert_eq!(nonce_at_genesis, 0, "Nonce at genesis block should be 0");

    // Current should be > 0
    assert!(
        nonce_at_latest > 0,
        "Latest nonce should be > 0 after transaction"
    );

    // KEY: genesis and latest MUST differ
    assert_ne!(
        nonce_at_genesis, nonce_at_latest,
        "BUG: nonce(block_0) == nonce(latest) - genesis query returns current state"
    );

    Ok(())
}

/// Verify "earliest" is exactly equivalent to block 0.
/// This catches bugs where "earliest" has different semantics than block 0.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_earliest_equals_block_zero() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    let address = client.address();

    // Query both ways
    let nonce_earliest = nonce_at_tag(&client, address, "earliest").await;
    let nonce_block_0 = nonce_at_number(&client, address, 0).await;

    // They should be identical
    assert_eq!(
        nonce_earliest, nonce_block_0,
        "BUG: 'earliest' != block 0 - earliest should be synonymous with block 0"
    );

    Ok(())
}

/// Verify nonce at receipt's block number reflects the transaction.
/// If tx is included in block N, nonce at block N should be post-tx nonce.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_receipt_block_reflects_tx() -> anyhow::Result<()> {
    let (_rollup, client) = setup_rollup().await;

    let address = client.address();

    // Get current nonce
    let nonce_before = nonce_at_tag(&client, address, "latest").await;

    // Send transaction and get receipt
    let tx_hash = client.send_eth(Address::ZERO, U256::from(0x9876)).await;
    let receipt = client.wait_for_finalized_receipt(tx_hash).await;

    // Get the block number from receipt
    let tx_block_number = receipt
        .block_number
        .expect("Receipt should have block number");

    // Query nonce at that exact block
    let nonce_at_tx_block = nonce_at_number(&client, address, tx_block_number).await;

    // The nonce at the tx's block should be AFTER the tx (nonce_before + 1)
    assert_eq!(
        nonce_at_tx_block,
        nonce_before + 1,
        "BUG: Nonce at tx's block ({tx_block_number}) should be post-tx nonce"
    );

    // Also verify the block before (if it exists) has pre-tx nonce
    if tx_block_number > 0 {
        let nonce_at_prev_block = nonce_at_number(&client, address, tx_block_number - 1).await;
        assert_eq!(
            nonce_at_prev_block, nonce_before,
            "BUG: Nonce at block before tx should be pre-tx nonce"
        );
    }

    Ok(())
}

/// Verify that "latest" without paused batches works correctly.
/// Send tx, wait for seal, verify latest reflects the sealed state.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_latest_after_seal() -> anyhow::Result<()> {
    let (rollup, client) = setup_rollup().await;
    // NOTE: Do NOT pause batches - test normal operation

    let address = client.address();
    let nonce_before = nonce_at_tag(&client, address, "latest").await;

    // Send and wait for finalization
    let tx_hash = client.send_eth(Address::ZERO, U256::from(0xABCD)).await;
    client.wait_for_finalized_receipt(tx_hash).await;
    rollup.wait_for_next_blocks(1).await;

    let nonce_after = nonce_at_tag(&client, address, "latest").await;

    assert_eq!(
        nonce_after,
        nonce_before + 1,
        "Latest nonce should increment after sealed transaction"
    );

    Ok(())
}

/// Verify behavior at the exact boundary between blocks.
/// Query at block N where tx was mined, and block N-1.
#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_count_block_boundary() -> anyhow::Result<()> {
    let (rollup, client) = setup_rollup().await;

    let address = client.address();

    // Record pre-tx block
    let pre_tx_block = client.block_number().await;
    let nonce_pre = nonce_at_number(&client, address, pre_tx_block).await;

    // Send tx and wait for finalization
    let tx_hash = client.send_eth(Address::ZERO, U256::from(0xBCDE)).await;
    let receipt = client.wait_for_finalized_receipt(tx_hash).await;
    rollup.wait_for_next_blocks(1).await;

    let tx_block = receipt
        .block_number
        .expect("Receipt should have block number");

    // If tx is in a new block, verify boundary
    if tx_block > pre_tx_block {
        let nonce_at_pre_block = nonce_at_number(&client, address, pre_tx_block).await;
        let nonce_at_tx_block = nonce_at_number(&client, address, tx_block).await;

        assert_eq!(
            nonce_at_pre_block, nonce_pre,
            "Nonce at pre-tx block should be unchanged"
        );
        assert_eq!(
            nonce_at_tx_block,
            nonce_pre + 1,
            "Nonce at tx block should be incremented"
        );

        // KEY: Must differ at boundary
        assert_ne!(
            nonce_at_pre_block, nonce_at_tx_block,
            "BUG: Nonce same at block boundary"
        );
    }

    Ok(())
}
