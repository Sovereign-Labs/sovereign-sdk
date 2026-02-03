//! Tests for eth_getBlockByNumber and eth_getBlockByHash RPC endpoints.
//!
//! Test case references are to docs/eth_getBlockBy_test_cases.md

use alloy_primitives::BlockHash;
use alloy_provider::{DynProvider, Provider};
use alloy_rpc_types_eth::BlockNumberOrTag::{self, Earliest, Finalized, Latest, Pending, Safe};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde_json::json;

use crate::evm::evm_test_helper::{
    alloy_client, create_simple_storage_client, deploy_contract_check, setup_test_rollup,
    EVM_EXTENSION, SENDER_PRIV_KEY,
};

async fn wait_for_pending_block_with_txs(
    client: &DynProvider,
) -> anyhow::Result<alloy_rpc_types_eth::Block> {
    use alloy_rpc_types_eth::BlockTransactions;

    for _ in 0..100 {
        let block = client.get_block_by_number(Pending).await?.unwrap();
        let tx_count = match &block.transactions {
            BlockTransactions::Hashes(h) => h.len(),
            BlockTransactions::Full(f) => f.len(),
            BlockTransactions::Uncle => 0,
        };
        if tx_count > 0 {
            return Ok(block);
        }
        tokio::task::yield_now().await;
    }

    anyhow::bail!("timed out waiting for pending block to include transactions");
}

// =============================================================================
// Group 1: Block Tag Semantics
// =============================================================================

/// TC01, TC05, TC06, TC08: Block tags earliest, safe, finalized
///
/// Verifies:
/// - `earliest` returns genesis (block 0)
/// - `safe` and `finalized` return the sealed head (this rollup's semantics)
/// - `safe` == `finalized` (both map to head)
#[tokio::test(flavor = "multi_thread")]
async fn test_block_tags_earliest_safe_finalized() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let sealed_head_number = client.get_block_number().await?;

    // TC01: earliest returns genesis
    let earliest = client.get_block_by_number(Earliest).await?.unwrap();
    assert_eq!(earliest.header.number, 0, "earliest should be block 0");

    // TC05: safe returns sealed head
    let safe_block = client.get_block_by_number(Safe).await?.unwrap();
    assert_eq!(
        safe_block.header.number, sealed_head_number,
        "safe should equal eth_blockNumber (sealed head)"
    );
    assert_ne!(
        safe_block.header.hash,
        BlockHash::ZERO,
        "safe block should have real hash"
    );

    // TC06: finalized returns sealed head
    let finalized_block = client.get_block_by_number(Finalized).await?.unwrap();
    assert_eq!(
        finalized_block.header.number, sealed_head_number,
        "finalized should equal eth_blockNumber (sealed head)"
    );
    assert_ne!(
        finalized_block.header.hash,
        BlockHash::ZERO,
        "finalized block should have real hash"
    );

    // TC08: safe == finalized (both map to head in this rollup)
    assert_eq!(
        safe_block.header.number, finalized_block.header.number,
        "safe and finalized should return same block number"
    );
    assert_eq!(
        safe_block.header.hash, finalized_block.header.hash,
        "safe and finalized should return same block hash"
    );

    Ok(())
}

/// TC03, TC04, TC07: Block tags latest and pending
///
/// Documents L1 divergence: In this rollup, latest == pending (both return pending block).
/// On Ethereum L1, `latest` returns the last sealed block, `pending` returns the block being built.
#[tokio::test(flavor = "multi_thread")]
async fn test_block_tags_latest_pending_equivalence() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(2).await;
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");
    let client = alloy_client(rollup.http_addr);
    rollup.pause_preferred_batches().await;
    let tx_hash = simple_storage.set_value(contract_address, 3000).await;
    simple_storage.wait_for_receipt(tx_hash).await;

    let sealed_head_number = client.get_block_number().await?;

    // TC03: latest returns pending block (L1 DIVERGENCE)
    let latest_block = client.get_block_by_number(Latest).await?.unwrap();

    // TC04: pending returns pending block
    let pending_block = client.get_block_by_number(Pending).await?.unwrap();

    // TC07: latest == pending (L1 DIVERGENCE: on L1 they would differ)
    assert_eq!(
        latest_block.header.number, pending_block.header.number,
        "L1 DIVERGENCE: latest and pending return same block"
    );
    assert_eq!(
        latest_block.header.hash, pending_block.header.hash,
        "L1 DIVERGENCE: latest and pending have same hash"
    );

    // Pending block number should be sealed_head + 1
    assert_eq!(
        pending_block.header.number,
        sealed_head_number + 1,
        "pending block number should be sealed_head + 1"
    );

    // Pending block has synthetic (non-zero) hash (L1 DIVERGENCE: L1 returns null)
    assert_ne!(
        pending_block.header.hash,
        BlockHash::ZERO,
        "pending block hash should be synthetic and non-zero"
    );
    let pending_by_hash = client
        .get_block_by_hash(pending_block.header.hash)
        .await?
        .unwrap();
    assert_eq!(
        pending_by_hash.header.number, pending_block.header.number,
        "pending block hash should be resolvable via eth_getBlockByHash"
    );

    Ok(())
}

/// TC40, TC41, TC43: Cross-endpoint consistency with eth_blockNumber
///
/// Verifies:
/// - eth_blockNumber == safe.number == finalized.number
/// - pending.number == eth_blockNumber + 1 when pending txs exist (otherwise equals sealed head)
#[tokio::test(flavor = "multi_thread")]
async fn test_block_number_consistency() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(3).await;
    let client = alloy_client(rollup.http_addr);
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");
    // Wait for deployment tx to be sealed before pausing
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let eth_block_number = client.get_block_number().await?;

    // TC40: eth_blockNumber == safe.number
    let safe_block = client.get_block_by_number(Safe).await?.unwrap();
    assert_eq!(
        safe_block.header.number, eth_block_number,
        "eth_blockNumber should equal safe block number"
    );

    // TC41: eth_blockNumber == finalized.number
    let finalized_block = client.get_block_by_number(Finalized).await?.unwrap();
    assert_eq!(
        finalized_block.header.number, eth_block_number,
        "eth_blockNumber should equal finalized block number"
    );

    let latest_block = client.get_block_by_number(Latest).await?.unwrap();
    assert_eq!(
        latest_block.header.number, eth_block_number,
        "latest block should equal sealed head when no pending txs exist"
    );

    // With no pending txs, pending falls back to latest sealed block
    let pending_block = client.get_block_by_number(Pending).await?.unwrap();
    assert_eq!(
        pending_block.header.number, eth_block_number,
        "pending block should equal sealed head when no pending txs exist"
    );

    // Create a pending tx and re-check latest/pending behavior
    let tx_hash = simple_storage.set_value(contract_address, 42).await;
    simple_storage.wait_for_receipt(tx_hash).await;

    let latest_block = client.get_block_by_number(Latest).await?.unwrap();
    let pending_block = client.get_block_by_number(Pending).await?.unwrap();
    assert_eq!(
        latest_block.header.number,
        eth_block_number + 1,
        "latest block should track pending when pending txs exist"
    );
    // TC43: pending.number == eth_blockNumber + 1 when pending txs exist
    assert_eq!(
        pending_block.header.number,
        eth_block_number + 1,
        "pending block number should be eth_blockNumber + 1 when pending txs exist"
    );

    Ok(())
}

/// TC10: Non-existent (future) block returns None
#[tokio::test(flavor = "multi_thread")]
async fn test_nonexistent_block_returns_none() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let current_block = client.get_block_number().await?;
    let future_block = current_block + 1000;

    let result = client
        .get_block_by_number(BlockNumberOrTag::Number(future_block))
        .await?;

    assert!(
        result.is_none(),
        "future block {future_block} should return None"
    );

    Ok(())
}

// =============================================================================
// Group 2: Sealed vs Pending Block Differences
// =============================================================================

/// TC12, TC16: Sealed block has real hash and non-zero size
#[tokio::test(flavor = "multi_thread")]
async fn test_sealed_block_has_real_hash() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let sealed_head_number = client.get_block_number().await?;
    let sealed_block = client
        .get_block_by_number(BlockNumberOrTag::Number(sealed_head_number))
        .await?
        .unwrap();

    // TC12: Sealed block has real (non-zero) hash
    assert_ne!(
        sealed_block.header.hash,
        BlockHash::ZERO,
        "sealed block should have non-zero hash"
    );

    // TC16: Sealed block has non-zero size
    let size = sealed_block.header.size.unwrap_or_default().to::<u64>();
    assert!(size > 0, "sealed block should have non-zero size");

    Ok(())
}

/// TC13, TC14, TC15, TC17, TC19: Pending block properties
///
/// Verifies:
/// - Pending hash is synthetic (L1 divergence: would be null)
/// - Pending number is sealed_head + 1
/// - Pending parentHash equals sealed head's hash
/// - Pending size is non-zero when pending txs exist
/// - Pending gasUsed reflects pending txs
#[tokio::test(flavor = "multi_thread")]
async fn test_pending_block_properties() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(2).await;
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");
    let client = alloy_client(rollup.http_addr);
    rollup.pause_preferred_batches().await;

    let sealed_head_number = client.get_block_number().await?;
    let sealed_block = client
        .get_block_by_number(BlockNumberOrTag::Number(sealed_head_number))
        .await?
        .unwrap();
    let tx_hash = simple_storage.set_value(contract_address, 3000).await;
    simple_storage.wait_for_receipt(tx_hash).await;
    let pending_block = client.get_block_by_number(Pending).await?.unwrap();

    // TC13: Pending hash is synthetic (L1 DIVERGENCE: L1 returns null)
    assert_ne!(
        pending_block.header.hash,
        BlockHash::ZERO,
        "pending block hash should be synthetic and non-zero"
    );
    let pending_by_hash = client
        .get_block_by_hash(pending_block.header.hash)
        .await?
        .unwrap();
    assert_eq!(
        pending_by_hash.header.number, pending_block.header.number,
        "pending block hash should be resolvable via eth_getBlockByHash"
    );

    // TC14: Pending number is sealed_head + 1
    assert_eq!(
        pending_block.header.number,
        sealed_head_number + 1,
        "pending block number should be sealed_head + 1"
    );

    // TC15: Pending parentHash equals sealed head's hash
    assert_eq!(
        pending_block.header.parent_hash, sealed_block.header.hash,
        "pending parentHash should equal sealed head's hash"
    );

    // TC17: Pending size is non-zero when pending txs exist
    let pending_size = pending_block.header.size.unwrap_or_default().to::<u64>();
    assert!(
        pending_size > 0,
        "pending block size should be non-zero when pending txs exist"
    );

    // TC19: Pending gasUsed reflects pending transactions
    assert!(
        pending_block.header.gas_used > 0,
        "pending block gasUsed should be non-zero when pending txs exist"
    );

    Ok(())
}

/// TC17, TC19: Pending block without txs falls back to latest sealed block
#[tokio::test(flavor = "multi_thread")]
async fn test_pending_without_txs_falls_back_to_sealed() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let sealed_head_number = client.get_block_number().await?;
    let sealed_block = client
        .get_block_by_number(BlockNumberOrTag::Number(sealed_head_number))
        .await?
        .unwrap();
    let pending_block = client.get_block_by_number(Pending).await?.unwrap();

    assert_eq!(
        pending_block.header.number, sealed_block.header.number,
        "pending should fall back to latest sealed block when no pending txs exist"
    );
    assert_eq!(
        pending_block.header.hash, sealed_block.header.hash,
        "pending should fall back to latest sealed block hash when no pending txs exist"
    );

    Ok(())
}

// =============================================================================
// Group 3: Transaction Serialization (details parameter)
// =============================================================================

/// TC22, TC23, TC24: Transaction hashes mode vs full mode
///
/// Verifies:
/// - `details=false` (default) returns transaction hashes
/// - `details=true` returns full transaction objects
/// - Transaction count matches in both modes
#[tokio::test(flavor = "multi_thread")]
async fn test_transactions_hashes_vs_full() -> anyhow::Result<()> {
    use alloy_primitives::utils::parse_ether;
    use alloy_primitives::Address;
    use alloy_rpc_types_eth::BlockTransactions;
    use sov_evm_test_utils::{Erc20, Submit};

    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;

    // Deploy contract and mint (creates 2 txs)
    let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;
    // Pause before submitting so txs remain pending
    rollup.pause_preferred_batches().await;
    usdc.mint(Address::ZERO, parse_ether("1")?).submit().await?;

    // TC22: Get pending block with hashes only (default)
    let block_hashes = wait_for_pending_block_with_txs(&client).await?;

    // TC23: Get pending block with full transactions
    let block_full = client.get_block_by_number(Pending).full().await?.unwrap();

    let pending_number = block_hashes.header.number;

    // TC24: Transaction count matches in both modes
    let hashes_count = match &block_hashes.transactions {
        BlockTransactions::Hashes(h) => h.len(),
        BlockTransactions::Full(f) => f.len(),
        BlockTransactions::Uncle => 0,
    };
    let full_count = match &block_full.transactions {
        BlockTransactions::Hashes(h) => h.len(),
        BlockTransactions::Full(f) => f.len(),
        BlockTransactions::Uncle => 0,
    };

    assert_eq!(
        hashes_count, full_count,
        "transaction count should match in both modes"
    );
    assert!(hashes_count > 0, "expected at least 1 transaction in block");

    // Verify hashes mode returns hashes
    let hashes = match &block_hashes.transactions {
        BlockTransactions::Hashes(h) => h,
        _ => panic!("expected hashes mode to return BlockTransactions::Hashes"),
    };

    // Verify full mode returns full transactions
    let full_txs = match &block_full.transactions {
        BlockTransactions::Full(txs) => txs,
        _ => panic!("expected full mode to return BlockTransactions::Full"),
    };

    for (i, tx) in full_txs.iter().enumerate() {
        // TC25-TC29: Validate full tx object fields
        assert_ne!(
            tx.inner.hash(),
            &BlockHash::ZERO,
            "tx {i} should have non-zero hash"
        );
        assert!(tx.block_number.is_some(), "tx {i} should have blockNumber");
        assert_eq!(
            tx.block_number.unwrap(),
            pending_number,
            "tx {i} blockNumber should match block"
        );
        assert_eq!(
            tx.transaction_index,
            Some(i as u64),
            "tx {i} should have correct transactionIndex"
        );
        assert_eq!(
            tx.block_hash,
            Some(block_full.header.hash),
            "tx {i} blockHash should match block hash"
        );
    }

    // TC29: Hashes mode matches full tx hashes
    for (i, hash) in hashes.iter().enumerate() {
        assert_eq!(
            hash,
            full_txs[i].inner.hash(),
            "hashes[{i}] should match full tx hash"
        );
    }

    Ok(())
}

/// TC30: Empty block returns empty transactions array in both modes
#[tokio::test(flavor = "multi_thread")]
async fn test_empty_block_transactions() -> anyhow::Result<()> {
    use alloy_rpc_types_eth::BlockTransactions;

    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    // Block 0 (genesis) should have no transactions
    let block_hashes = client
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await?
        .unwrap();
    let block_full = client
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .full()
        .await?
        .unwrap();

    let hashes_empty = match &block_hashes.transactions {
        BlockTransactions::Hashes(h) => h.is_empty(),
        BlockTransactions::Full(f) => f.is_empty(),
        BlockTransactions::Uncle => true,
    };
    let full_empty = match &block_full.transactions {
        BlockTransactions::Hashes(h) => h.is_empty(),
        BlockTransactions::Full(f) => f.is_empty(),
        BlockTransactions::Uncle => true,
    };

    assert!(
        hashes_empty,
        "genesis block should have no transactions (hashes mode)"
    );
    assert!(
        full_empty,
        "genesis block should have no transactions (full mode)"
    );

    Ok(())
}

// =============================================================================
// Group 4: eth_getBlockByHash
// =============================================================================

/// TC32, TC36: Get sealed block by hash - round-trip consistency
#[tokio::test(flavor = "multi_thread")]
async fn test_get_block_by_hash_sealed() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let sealed_head = client.get_block_number().await?;
    let block_by_number = client
        .get_block_by_number(BlockNumberOrTag::Number(sealed_head))
        .await?
        .unwrap();

    let hash = block_by_number.header.hash;
    assert_ne!(hash, BlockHash::ZERO, "sealed block should have real hash");

    // TC32: Get block by hash
    let block_by_hash = client.get_block_by_hash(hash).await?.unwrap();

    // TC36: Round-trip - all fields should match
    assert_eq!(
        block_by_number.header.number, block_by_hash.header.number,
        "block number should match"
    );
    assert_eq!(
        block_by_number.header.hash, block_by_hash.header.hash,
        "block hash should match"
    );
    assert_eq!(
        block_by_number.header.parent_hash, block_by_hash.header.parent_hash,
        "parent hash should match"
    );
    assert_eq!(
        block_by_number.header.timestamp, block_by_hash.header.timestamp,
        "timestamp should match"
    );
    assert_eq!(
        block_by_number.header.gas_used, block_by_hash.header.gas_used,
        "gas_used should match"
    );
    assert_eq!(
        block_by_number.header.gas_limit, block_by_hash.header.gas_limit,
        "gas_limit should match"
    );

    Ok(())
}

/// TC35: `details` param works with eth_getBlockByHash (full tx objects)
#[tokio::test(flavor = "multi_thread")]
async fn test_get_block_by_hash_full_transactions() -> anyhow::Result<()> {
    use alloy_primitives::utils::parse_ether;
    use alloy_primitives::Address;
    use alloy_rpc_types_eth::BlockTransactions;
    use sov_evm_test_utils::{Erc20, Submit};

    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;

    // Create a sealed block with transactions
    let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;
    usdc.mint(Address::ZERO, parse_ether("1")?).submit().await?;
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let sealed_head = client.get_block_number().await?;
    let block_by_number_full = client
        .get_block_by_number(BlockNumberOrTag::Number(sealed_head))
        .full()
        .await?
        .unwrap();

    // Default (details=false) should return hashes
    let block_by_hash_hashes = client
        .get_block_by_hash(block_by_number_full.header.hash)
        .await?
        .unwrap();
    match &block_by_hash_hashes.transactions {
        BlockTransactions::Hashes(_) => {}
        _ => panic!("expected hashes mode from eth_getBlockByHash"),
    }

    // details=true should return full transactions
    let block_by_hash_full = client
        .get_block_by_hash(block_by_number_full.header.hash)
        .full()
        .await?
        .unwrap();
    let hashes = match &block_by_hash_hashes.transactions {
        BlockTransactions::Hashes(h) => h,
        _ => panic!("expected hashes mode from eth_getBlockByHash"),
    };
    let full_txs = match &block_by_hash_full.transactions {
        BlockTransactions::Full(txs) => txs,
        _ => panic!("expected full mode from eth_getBlockByHash"),
    };
    let expected_count = match &block_by_number_full.transactions {
        BlockTransactions::Full(txs) => txs.len(),
        _ => panic!("expected full mode from eth_getBlockByNumber"),
    };
    assert_eq!(
        full_txs.len(),
        expected_count,
        "full tx count should match block by number"
    );

    // Cross-check: hashes[i] == full[i].hash
    for (i, hash) in hashes.iter().enumerate() {
        assert_eq!(
            hash,
            full_txs[i].inner.hash(),
            "eth_getBlockByHash: hashes[{i}] should match full tx hash"
        );
    }

    Ok(())
}

/// TC33: Non-existent hash returns None
#[tokio::test(flavor = "multi_thread")]
async fn test_get_block_by_hash_nonexistent() -> anyhow::Result<()> {
    use alloy_primitives::B256;

    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    // Random hash that doesn't exist
    let random_hash = B256::repeat_byte(0xAB);
    let result = client.get_block_by_hash(random_hash).await?;

    assert!(result.is_none(), "non-existent hash should return None");

    Ok(())
}

/// TC34: Zero hash returns None (pending block cannot be fetched by hash)
#[tokio::test(flavor = "multi_thread")]
async fn test_get_block_by_hash_zero() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    // Zero hash is not a valid block hash and should return None
    let result = client.get_block_by_hash(BlockHash::ZERO).await?;

    assert!(result.is_none(), "zero hash should return None");

    Ok(())
}

// =============================================================================
// Group 5: EIP-1898 blockHash object
// =============================================================================

/// TC46-TC49: EIP-1898 blockHash object support
#[tokio::test(flavor = "multi_thread")]
async fn test_eip1898_block_hash_object() -> anyhow::Result<()> {
    use alloy_primitives::B256;
    use alloy_rpc_types_eth::Block;

    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let rpc_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    rollup.wait_for_next_blocks(2).await;
    rollup.pause_preferred_batches().await;

    let sealed_head = client.get_block_number().await?;
    let block = client
        .get_block_by_number(BlockNumberOrTag::Number(sealed_head))
        .await?
        .unwrap();
    let block_hash = format!("{:#x}", block.header.hash);

    // TC46: blockHash object
    let block_from_obj: Option<Block> = rpc_client
        .ws
        .request(
            "eth_getBlockByNumber",
            rpc_params![json!({ "blockHash": block_hash.clone() }), false],
        )
        .await
        .unwrap();
    let block_from_obj = block_from_obj.expect("blockHash object should resolve");
    assert_eq!(
        block_from_obj.header.hash, block.header.hash,
        "blockHash object should return the same block"
    );

    // TC47/TC48: requireCanonical true/false
    for require_canonical in [true, false] {
        let block_from_obj: Option<Block> = rpc_client
            .ws
            .request(
                "eth_getBlockByNumber",
                rpc_params![
                    json!({ "blockHash": block_hash.clone(), "requireCanonical": require_canonical }),
                    false
                ],
            )
            .await
            .unwrap();
        let block_from_obj = block_from_obj.expect("blockHash object should resolve");
        assert_eq!(
            block_from_obj.header.hash, block.header.hash,
            "blockHash object should return the same block"
        );
    }

    // TC49: Non-existent hash returns None
    let random_hash = B256::repeat_byte(0xAB);
    let random_hash = format!("{:#x}", random_hash);
    let missing: Option<Block> = rpc_client
        .ws
        .request(
            "eth_getBlockByNumber",
            rpc_params![json!({ "blockHash": random_hash }), false],
        )
        .await
        .unwrap();
    assert!(
        missing.is_none(),
        "non-existent blockHash should return None"
    );

    Ok(())
}

// =============================================================================
// Group 6: Parent Hash Chain Integrity
// =============================================================================

/// TC37, TC38, TC39: Parent hash chain is valid
#[tokio::test(flavor = "multi_thread")]
async fn test_parent_hash_chain_integrity() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(5).await;
    rollup.pause_preferred_batches().await;

    let sealed_head = client.get_block_number().await?;
    assert!(sealed_head >= 4, "need at least 5 blocks for this test");

    // TC38: Genesis block parentHash is zero
    let genesis = client
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await?
        .unwrap();
    assert_eq!(
        genesis.header.parent_hash,
        BlockHash::ZERO,
        "genesis parentHash should be zero"
    );

    // TC37, TC39: Verify chain from block 1 to sealed_head
    let mut prev_hash = genesis.header.hash;
    for n in 1..=sealed_head {
        let block = client
            .get_block_by_number(BlockNumberOrTag::Number(n))
            .await?
            .unwrap();

        assert_eq!(
            block.header.parent_hash,
            prev_hash,
            "block {} parentHash should equal block {} hash",
            n,
            n - 1
        );

        prev_hash = block.header.hash;
    }

    Ok(())
}

// =============================================================================
// Group 7: Value Cross-Checks
// =============================================================================

/// TC108-TC112: Block receipts cross-check
///
/// Verifies:
/// - Receipt count matches transaction count
/// - All receipts have matching blockHash and blockNumber
/// - Block gasUsed matches sum of receipt gasUsed
#[tokio::test(flavor = "multi_thread")]
async fn test_block_receipts_cross_check() -> anyhow::Result<()> {
    use alloy_primitives::utils::parse_ether;
    use alloy_primitives::Address;
    use alloy_rpc_types_eth::{BlockId, BlockTransactions};
    use sov_evm_test_utils::{Erc20, Submit};

    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;

    // Deploy contract and mint (creates 2 txs)
    let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;
    usdc.mint(Address::ZERO, parse_ether("1")?).submit().await?;

    rollup.wait_for_next_blocks(1).await;
    rollup.pause_preferred_batches().await;

    let sealed_head = client.get_block_number().await?;
    let block = client
        .get_block_by_number(BlockNumberOrTag::Number(sealed_head))
        .await?
        .unwrap();

    let receipts = client
        .get_block_receipts(BlockId::Number(BlockNumberOrTag::Number(sealed_head)))
        .await?
        .unwrap();

    // TC108: Receipt count matches tx count
    let tx_count = match &block.transactions {
        BlockTransactions::Hashes(h) => h.len(),
        BlockTransactions::Full(f) => f.len(),
        BlockTransactions::Uncle => 0,
    };
    assert_eq!(
        receipts.len(),
        tx_count,
        "receipt count should match transaction count"
    );

    // TC109, TC110, TC111: All receipts have correct block info
    let mut total_gas_used = 0u64;
    for (i, receipt) in receipts.iter().enumerate() {
        // TC109: blockHash matches
        if let Some(receipt_block_hash) = receipt.block_hash {
            assert_eq!(
                receipt_block_hash, block.header.hash,
                "receipt {i} blockHash should match block hash"
            );
        }

        // TC110: blockNumber matches
        if let Some(receipt_block_number) = receipt.block_number {
            assert_eq!(
                receipt_block_number, sealed_head,
                "receipt {i} blockNumber should match"
            );
        }

        // TC111: transactionIndex is sequential
        assert_eq!(
            receipt.transaction_index,
            Some(i as u64),
            "receipt {i} transactionIndex should be {i}"
        );

        total_gas_used += receipt.gas_used;
    }

    // TC112: Block gasUsed matches sum of receipt gasUsed
    assert_eq!(
        block.header.gas_used, total_gas_used,
        "block gasUsed should equal sum of receipt gasUsed"
    );

    Ok(())
}

/// TC45, TC141: Timestamp monotonicity
#[tokio::test(flavor = "multi_thread")]
async fn test_timestamp_monotonicity() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(5).await;
    rollup.pause_preferred_batches().await;

    let sealed_head = client.get_block_number().await?;

    let mut prev_timestamp = 0u64;
    for n in 0..=sealed_head {
        let block = client
            .get_block_by_number(BlockNumberOrTag::Number(n))
            .await?
            .unwrap();

        assert!(
            block.header.timestamp >= prev_timestamp,
            "block {} timestamp {} should be >= block {} timestamp {}",
            n,
            block.header.timestamp,
            n - 1,
            prev_timestamp
        );

        // TC141: Timestamp should be reasonable (non-zero after genesis)
        if n > 0 {
            assert!(
                block.header.timestamp > 0,
                "block {n} should have non-zero timestamp"
            );
        }

        prev_timestamp = block.header.timestamp;
    }

    Ok(())
}
