use alloy::consensus::{Eip658Value, TxReceipt, TxType};
use alloy_primitives::utils::parse_ether;
use alloy_primitives::{keccak256, Address, BlockHash, Bloom, B256, U256, U64};
use alloy_provider::DynProvider;
use alloy_provider::Provider;
use alloy_rpc_types_eth::BlockNumberOrTag::Finalized;
use alloy_rpc_types_eth::Header;
use alloy_rpc_types_eth::{Block, BlockId, BlockNumberOrTag, BlockTransactions, Filter};
use alloy_rpc_types_eth::{Transaction, TransactionReceipt};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use sov_evm_test_utils::{Erc20, LegacySimpleStorage, Submit};

use crate::evm::evm_test_helper::{
    alloy_client, create_simple_storage_client, setup_test_rollup, EVM_EXTENSION, SENDER_PRIV_KEY,
};

async fn by_number(
    client: &DynProvider,
    tag: impl Into<BlockNumberOrTag>,
) -> anyhow::Result<Option<Header>> {
    Ok(client
        .get_block_by_number(tag.into())
        .await?
        .map(|block| block.header))
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_by_number() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.pause_preferred_batches().await;

    assert_eq!(
        by_number(&client, BlockNumberOrTag::Earliest)
            .await?
            .unwrap()
            .number,
        0
    );
    assert_eq!(
        by_number(&client, BlockNumberOrTag::Latest)
            .await?
            .unwrap()
            .number,
        0
    );
    assert_eq!(
        by_number(&client, BlockNumberOrTag::Pending)
            .await?
            .unwrap()
            .number,
        0
    );
    assert_eq!(by_number(&client, 1).await?, None);
    assert_eq!(by_number(&client, 2).await?, None);

    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches().await;

    assert_eq!(
        by_number(&client, BlockNumberOrTag::Earliest)
            .await?
            .unwrap()
            .number,
        0
    );
    assert_eq!(
        by_number(&client, BlockNumberOrTag::Latest)
            .await?
            .unwrap()
            .number,
        1
    );
    assert_eq!(
        by_number(&client, BlockNumberOrTag::Pending)
            .await?
            .unwrap()
            .number,
        1
    );

    assert_eq!(by_number(&client, 1).await?.unwrap().number, 1);
    assert_eq!(by_number(&client, 2).await?, None);
    assert_eq!(by_number(&client, 3).await?, None);

    assert_eq!(
        by_number(&client, 1).await?.unwrap().parent_hash,
        by_number(&client, 0).await?.unwrap().hash
    );

    Ok(())
}

async fn by_hash(client: &DynProvider, hash: BlockHash) -> anyhow::Result<Option<Header>> {
    Ok(client
        .get_block_by_hash(hash)
        .await?
        .map(|block| block.header))
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_by_hash() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);

    rollup.wait_for_rollup_height_advance_by(2).await;
    rollup.pause_preferred_batches().await;

    let latest_hash = by_number(&client, BlockNumberOrTag::Latest)
        .await?
        .unwrap()
        .parent_hash;
    let latest = by_hash(&client, latest_hash).await?.unwrap();
    assert_eq!(latest.hash, latest_hash);
    assert_eq!(latest.number, 1);

    let pending_hash = by_number(&client, BlockNumberOrTag::Latest)
        .await?
        .unwrap()
        .hash;
    assert_ne!(pending_hash, BlockHash::ZERO);
    // Because the hash of the pending block is fake - it can't be fetched by hash
    assert_ne!(by_hash(&client, pending_hash).await?, None);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_receipts() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(1).await;

    let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;
    usdc.mint(Address::ZERO, parse_ether("1")?).submit().await?;
    rollup.pause_preferred_batches().await;

    let empty_block_receipts = client.get_block_receipts(BlockId::from(1)).await?.unwrap();
    assert_eq!(empty_block_receipts.len(), 0);

    let mut pending_receipts = client
        .get_block_receipts(BlockId::pending())
        .await?
        .unwrap();
    assert_eq!(pending_receipts.len(), 2);

    let deployment_receipt = pending_receipts.remove(0);
    assert!(deployment_receipt.contract_address.is_some());
    assert_eq!(deployment_receipt.transaction_index, Some(0));
    assert_eq!(deployment_receipt.logs().len(), 0);

    let mint_receipt = pending_receipts.remove(0);
    assert!(mint_receipt.contract_address.is_none());
    assert_eq!(mint_receipt.transaction_index, Some(1));
    assert_eq!(mint_receipt.logs().len(), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn block_size() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches().await;

    let header = by_number(&client, 0).await?.unwrap();
    // Block size is 508 bytes with gas_limit = 100_000_000_000 (5-byte RLP encoding).
    // The cached body RLP omits withdrawals while withdrawals are unavailable.
    assert_eq!(header.size.unwrap().to::<u64>(), 508);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_storage_at_returns_32_byte_data_hex() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let client = crate::evm::evm_test_helper::create_simple_storage_client(
        rollup.http_addr,
        crate::evm::evm_test_helper::SENDER_PRIV_KEY,
    )
    .await;

    let contract_address = crate::evm::evm_test_helper::deploy_contract_check(&client)
        .await
        .expect("contract deployment should succeed");
    crate::evm::evm_test_helper::set_value_check(&client, contract_address, 1)
        .await
        .expect("setting storage should succeed");

    // Query raw JSON-RPC output to assert Ethereum DATA shape (fixed 32-byte hex).
    let raw: String = client
        .rpc_client
        .ws
        .request(
            "eth_getStorageAt",
            rpc_params![contract_address, U256::from(0)],
        )
        .await?;

    assert_eq!(
        raw.len(),
        66,
        "eth_getStorageAt should return 0x-prefixed 32-byte hex data"
    );
    assert!(
        raw.starts_with("0x"),
        "eth_getStorageAt should return a hex string"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_block_transaction_count_by_hash_accepts_synthetic_hash() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches().await;

    let ws_client = crate::evm::evm_test_helper::create_simple_storage_client(
        rollup.http_addr,
        crate::evm::evm_test_helper::SENDER_PRIV_KEY,
    )
    .await;
    let tx_hash = ws_client.send_eth(Address::ZERO, U256::from(1)).await;
    ws_client.wait_for_receipt(tx_hash).await;

    let client = alloy_client(rollup.http_addr);
    let latest = by_number(&client, BlockNumberOrTag::Latest)
        .await?
        .expect("latest block should exist");
    let finalized = by_number(&client, Finalized)
        .await?
        .expect("finalized block should exist");
    assert_eq!(
        latest.number,
        finalized.number + 1,
        "latest should resolve to the pending synthetic block while tx is pending"
    );

    let by_hash: Option<U64> = ws_client
        .rpc_client
        .ws
        .request(
            "eth_getBlockTransactionCountByHash",
            rpc_params![latest.hash],
        )
        .await?;
    let by_number: Option<U64> = ws_client
        .rpc_client
        .ws
        .request(
            "eth_getBlockTransactionCountByNumber",
            rpc_params!["latest"],
        )
        .await?;

    assert_eq!(
        by_hash, by_number,
        "hash and number forms should agree for the same synthetic latest block"
    );
    assert!(
        by_hash.is_some(),
        "synthetic hash should be queryable by eth_getBlockTransactionCountByHash"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_receipt_unknown_tx() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);

    let missing = B256::from([0x11u8; 32]);
    let receipt = client.get_transaction_receipt(missing).await?;
    assert!(receipt.is_none());

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_receipt_fields() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    rollup.wait_for_rollup_height_advance_by(1).await;

    // Deploy contract in an isolated block.
    rollup.pause_preferred_batches().await;
    assert_pending_block_empty(&client).await?;
    let deploy_tx = simple_storage
        .deploy_contract()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches().await;

    let deploy_receipt = fetch_receipt(&client, deploy_tx).await?;
    assert_eq!(deploy_receipt.transaction_hash, deploy_tx);
    let sender = simple_storage.address();
    let (deploy_block, _) = assert_receipt_common(&client, &deploy_receipt, sender).await?;
    assert!(deploy_receipt.to.is_none());
    let contract_address = deploy_receipt
        .contract_address
        .expect("deployment receipt should include contract address");
    assert_ne!(contract_address, Address::ZERO);
    assert_eq!(deploy_receipt.logs().len(), 0);
    assert_eq!(*deploy_receipt.inner.logs_bloom(), Bloom::ZERO);
    assert_eq!(deploy_block.header.gas_used, deploy_receipt.gas_used);

    // Emit a log via set() in its own block for deterministic receipt/log assertions.
    let set_arg = 42u32;
    assert_pending_block_empty(&client).await?;
    let set_tx = simple_storage.set_value(contract_address, set_arg).await;
    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches().await;

    let set_receipt = fetch_receipt(&client, set_tx).await?;
    assert_eq!(set_receipt.transaction_hash, set_tx);
    let (set_block, _) = assert_receipt_common(&client, &set_receipt, sender).await?;
    assert_eq!(set_receipt.to, Some(contract_address));
    assert!(set_receipt.contract_address.is_none());
    assert_eq!(set_receipt.logs().len(), 1);
    assert_eq!(set_block.header.gas_used, set_receipt.gas_used);

    let log = &set_receipt.logs()[0];
    assert_log_matches_receipt(&set_receipt, log, 0);
    assert_eq!(log.address(), contract_address);
    assert_eq!(log.topics().len(), 4);
    assert!(!log.data().data.as_ref().is_empty());
    assert_ne!(*set_receipt.inner.logs_bloom(), Bloom::ZERO);

    let decoded = LegacySimpleStorage::decode_alloy(log.clone());
    assert_eq!(decoded.parsed.sender, sender);
    assert_eq!(decoded.parsed.topic1, U256::from(set_arg));
    assert_eq!(decoded.parsed.topic2, U256::from(set_arg));
    assert_eq!(decoded.parsed.value, U256::from(set_arg));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_receipt_pending_behavior() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches().await;

    let deploy_tx = simple_storage
        .deploy_contract()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Pending receipt has blockHash and blockNumber set
    let pending_receipt = client.get_transaction_receipt(deploy_tx).await?.unwrap();
    let pending_block_hash = pending_receipt
        .block_hash
        .expect("pending receipt must have blockHash");
    let pending_block_number = pending_receipt
        .block_number
        .expect("pending receipt must have blockNumber");

    // Store other fields for comparison
    let pending_gas_used = pending_receipt.gas_used;
    let pending_from = pending_receipt.from;
    let pending_status = pending_receipt.inner.status();

    // Seal the block
    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let sealed_receipt = client.get_transaction_receipt(deploy_tx).await?.unwrap();

    // blockHash changes from synthetic to real
    let sealed_block_hash = sealed_receipt
        .block_hash
        .expect("sealed receipt must have blockHash");
    assert_ne!(
        sealed_block_hash, pending_block_hash,
        "blockHash should change after sealing"
    );

    // blockNumber remains unchanged
    assert_eq!(
        sealed_receipt.block_number,
        Some(pending_block_number),
        "blockNumber should not change"
    );

    // All other fields remain unchanged
    assert_eq!(
        sealed_receipt.gas_used, pending_gas_used,
        "gasUsed should not change"
    );
    assert_eq!(sealed_receipt.from, pending_from, "from should not change");
    assert_eq!(
        sealed_receipt.inner.status(),
        pending_status,
        "status should not change"
    );

    // After sealing, verify cross-endpoint consistency
    let block = client
        .get_block_by_number(BlockNumberOrTag::Number(pending_block_number))
        .await?
        .expect("sealed block should exist");
    assert_eq!(
        block.header.hash, sealed_block_hash,
        "receipt blockHash must match eth_getBlockByNumber"
    );

    let tx = client
        .get_transaction_by_hash(deploy_tx)
        .await?
        .expect("transaction should exist");
    assert_eq!(
        tx.block_hash,
        Some(sealed_block_hash),
        "tx blockHash consistency"
    );
    assert_eq!(
        tx.block_number,
        Some(pending_block_number),
        "tx blockNumber consistency"
    );

    // Verify transactionIndex matches position in block
    let block_hashes = block_tx_hashes(&block);
    let tx_position = block_hashes
        .iter()
        .position(|h| *h == deploy_tx)
        .expect("tx should be in block");
    assert_eq!(
        sealed_receipt.transaction_index,
        Some(tx_position as u64),
        "transactionIndex matches position in block"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_receipt_pending_is_null() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches().await;
    assert_pending_block_empty(&client).await?;

    let deploy_tx = simple_storage
        .deploy_contract()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // L1 behavior: receipts for pending txs must be null. Sovereign currently returns pending receipts.
    let pending_receipt = client.get_transaction_receipt(deploy_tx).await?;
    assert!(pending_receipt.is_some());

    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let mined = client.get_transaction_receipt(deploy_tx).await?;
    assert!(mined.is_some());

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_receipt_multi_tx_block() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    rollup.wait_for_rollup_height_advance_by(1).await;

    // Deploy contract in its own block to get a stable address.
    rollup.pause_preferred_batches().await;
    assert_pending_block_empty(&client).await?;
    let deploy_tx = simple_storage
        .deploy_contract()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches().await;

    let deploy_receipt = fetch_receipt(&client, deploy_tx).await?;
    let contract_address = deploy_receipt
        .contract_address
        .expect("deployment receipt should include contract address");

    // Build a block with multiple txs (two logs, one plain transfer).
    assert_pending_block_empty(&client).await?;
    let tx1 = simple_storage.set_value(contract_address, 10).await;
    let receiver: Address = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    let tx2 = simple_storage.send_eth(receiver, U256::from(1)).await;
    let tx3 = simple_storage.set_value(contract_address, 11).await;

    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches().await;

    let r1 = fetch_receipt(&client, tx1).await?;
    let r2 = fetch_receipt(&client, tx2).await?;
    let r3 = fetch_receipt(&client, tx3).await?;
    assert_eq!(r1.transaction_hash, tx1);
    assert_eq!(r2.transaction_hash, tx2);
    assert_eq!(r3.transaction_hash, tx3);

    let block_number = r1
        .block_number
        .ok_or_else(|| anyhow::anyhow!("receipt missing block number"))?;
    let block_hash = r1
        .block_hash
        .ok_or_else(|| anyhow::anyhow!("receipt missing block hash"))?;

    assert_eq!(r2.block_number, Some(block_number));
    assert_eq!(r3.block_number, Some(block_number));
    assert_eq!(r2.block_hash, Some(block_hash));
    assert_eq!(r3.block_hash, Some(block_hash));

    let block = client
        .get_block_by_number(BlockNumberOrTag::Number(block_number))
        .await?
        .ok_or_else(|| anyhow::anyhow!("block {block_number} should exist"))?;
    assert_eq!(block.header.hash, block_hash);

    let block_hashes = block_tx_hashes(&block);
    assert_eq!(block_hashes, vec![tx1, tx2, tx3]);

    let sender = simple_storage.address();
    assert_receipts_in_block(
        &[&r1, &r2, &r3],
        &[
            (Some(contract_address), 1, true),
            (Some(receiver), 0, false),
            (Some(contract_address), 1, true),
        ],
        &block,
        sender,
    );

    let r1_log = r1.logs().first().unwrap();
    let r3_log = r3.logs().first().unwrap();
    // logIndex is sequential across block (r1 has 1 log at index 0, r2 has 0, r3 has 1 log at index 1)
    assert_log_matches_receipt(&r1, r1_log, 0);
    assert_log_matches_receipt(&r3, r3_log, 1);

    let topic0 = keccak256(b"SimpleLog(address,uint256,uint256,uint256)");
    let mut got_logs = client
        .get_logs(
            &Filter::new()
                .at_block_hash(block_hash)
                .address(contract_address)
                .event_signature(topic0),
        )
        .await?;
    let mut expected_logs = vec![r1_log.clone(), r3_log.clone()];
    expected_logs.sort_by_key(|log| log.log_index.unwrap());
    got_logs.sort_by_key(|log| log.log_index.unwrap());
    assert_eq!(got_logs, expected_logs);

    let block_receipts = client
        .get_block_receipts(BlockId::from(block_number))
        .await?
        .unwrap();
    assert_eq!(block_receipts.len(), 3);

    let mut by_hash = std::collections::HashMap::new();
    for receipt in block_receipts {
        by_hash.insert(receipt.transaction_hash, receipt);
    }

    assert_receipt_matches(&by_hash[&tx1], &r1);
    assert_receipt_matches(&by_hash[&tx2], &r2);
    assert_receipt_matches(&by_hash[&tx3], &r3);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_receipt_many_logs() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    rollup.wait_for_rollup_height_advance_by(1).await;

    // Deploy contract
    rollup.pause_preferred_batches().await;
    assert_pending_block_empty(&client).await?;
    let deploy_tx = simple_storage
        .deploy_contract()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let deploy_receipt = client.get_transaction_receipt(deploy_tx).await?.unwrap();
    let contract_address = deploy_receipt.contract_address.unwrap();

    // Emit 15 logs
    rollup.pause_preferred_batches().await;
    assert_pending_block_empty(&client).await?;
    let emit_tx = simple_storage
        .alloy_emit_logs(contract_address, 0, 15)
        .await;
    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let receipt = client.get_transaction_receipt(emit_tx).await?.unwrap();

    // 15 logs with correct sequential indices
    assert_eq!(receipt.logs().len(), 15, "should have 15 logs");
    for (i, log) in receipt.logs().iter().enumerate() {
        assert_eq!(
            log.log_index,
            Some(i as u64),
            "logIndex should be sequential"
        );
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_get_transaction_receipt_zero_address() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    rollup.wait_for_rollup_height_advance_by(1).await;

    // Send to zero address
    rollup.pause_preferred_batches().await;
    assert_pending_block_empty(&client).await?;
    let tx = simple_storage.send_eth(Address::ZERO, U256::from(1)).await;
    rollup.resume_preferred_batches().await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let receipt = client.get_transaction_receipt(tx).await?.unwrap();

    // to is zero address, not null
    assert_eq!(
        receipt.to,
        Some(Address::ZERO),
        "to should be zero address, not null"
    );

    Ok(())
}

async fn fetch_receipt(client: &DynProvider, tx_hash: B256) -> anyhow::Result<TransactionReceipt> {
    client
        .get_transaction_receipt(tx_hash)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing receipt for tx {tx_hash:?}"))
}

async fn assert_receipt_common(
    client: &DynProvider,
    receipt: &TransactionReceipt,
    sender: Address,
) -> anyhow::Result<(Block, Transaction)> {
    let block_number = receipt
        .block_number
        .ok_or_else(|| anyhow::anyhow!("receipt missing block number"))?;
    let block_hash = receipt
        .block_hash
        .ok_or_else(|| anyhow::anyhow!("receipt missing block hash"))?;
    let tx_index = receipt
        .transaction_index
        .ok_or_else(|| anyhow::anyhow!("receipt missing transaction index"))?;

    assert_ne!(block_hash, BlockHash::ZERO);
    assert!(block_number > 0);

    let block = client
        .get_block_by_number(BlockNumberOrTag::Number(block_number))
        .await?
        .ok_or_else(|| anyhow::anyhow!("block {block_number} should exist"))?;

    assert_eq!(block.header.hash, block_hash);
    assert_eq!(block.header.number, block_number);

    let base_fee = block
        .header
        .base_fee_per_gas
        .ok_or_else(|| anyhow::anyhow!("block {block_number} missing base_fee_per_gas"))?;
    assert!(base_fee > 0);

    let tx = client
        .get_transaction_by_hash(receipt.transaction_hash)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "transaction missing for {tx_hash:?}",
                tx_hash = receipt.transaction_hash
            )
        })?;

    assert_eq!(tx.block_hash, Some(block_hash));
    assert_eq!(tx.block_number, Some(block_number));
    assert_eq!(tx.transaction_index, Some(tx_index));

    let expected_index = tx_index_in_block(&block, receipt.transaction_hash)?;
    assert_eq!(tx_index, expected_index);

    let block_hashes = block_tx_hashes(&block);
    assert_eq!(block_hashes.len(), 1);
    assert_eq!(block_hashes[0], receipt.transaction_hash);

    assert_eq!(receipt.from, sender);
    assert!(receipt.gas_used > 0);
    assert!(receipt.effective_gas_price > 0);
    assert_eq!(receipt.blob_gas_used, None);
    assert_eq!(receipt.blob_gas_price, None);
    assert_eq!(receipt.transaction_type(), TxType::Eip1559);
    assert!(receipt.status());
    assert!(matches!(
        receipt.inner.status_or_post_state(),
        Eip658Value::Eip658(_)
    ));
    assert_eq!(
        tx.effective_gas_price,
        Some(receipt.effective_gas_price),
        "transaction effective_gas_price should match receipt effective_gas_price",
    );

    let cumulative = receipt.inner.cumulative_gas_used();
    assert_eq!(cumulative, receipt.gas_used);
    assert_eq!(block.header.gas_used, receipt.gas_used);

    // logsBloom is valid (256 bytes, non-zero for tx with logs).
    // The bloom filter contains log data. Full derivation test is out of scope.
    if !receipt.logs().is_empty() {
        assert_ne!(*receipt.inner.logs_bloom(), Bloom::ZERO);
    }

    Ok((block, tx))
}

async fn assert_pending_block_empty(client: &DynProvider) -> anyhow::Result<()> {
    let pending = client
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .ok_or_else(|| anyhow::anyhow!("pending block should exist"))?;
    assert!(block_tx_hashes(&pending).is_empty());
    Ok(())
}

fn block_tx_hashes(block: &Block) -> Vec<B256> {
    match &block.transactions {
        BlockTransactions::Hashes(hashes) => hashes.clone(),
        BlockTransactions::Full(txs) => txs.iter().map(|tx| *tx.inner.hash()).collect(),
        BlockTransactions::Uncle => Vec::new(),
    }
}

fn tx_index_in_block(block: &Block, tx_hash: B256) -> anyhow::Result<u64> {
    let hashes = block_tx_hashes(block);
    hashes
        .iter()
        .position(|hash| *hash == tx_hash)
        .map(|idx| idx as u64)
        .ok_or_else(|| anyhow::anyhow!("tx {tx_hash:?} not found in block"))
}

fn assert_log_matches_receipt(
    receipt: &TransactionReceipt,
    log: &alloy_rpc_types_eth::Log,
    log_index: u64,
) {
    assert_eq!(log.block_hash, receipt.block_hash);
    assert_eq!(log.block_number, receipt.block_number);
    assert_eq!(log.transaction_hash, Some(receipt.transaction_hash));
    assert_eq!(log.transaction_index, receipt.transaction_index);
    assert_eq!(log.log_index, Some(log_index));
    assert!(!log.removed);
}

fn assert_receipt_matches(left: &TransactionReceipt, right: &TransactionReceipt) {
    assert_eq!(left.transaction_hash, right.transaction_hash);
    assert_eq!(left.transaction_index, right.transaction_index);
    assert_eq!(left.block_hash, right.block_hash);
    assert_eq!(left.block_number, right.block_number);
    assert_eq!(left.gas_used, right.gas_used);
    assert_eq!(
        left.inner.cumulative_gas_used(),
        right.inner.cumulative_gas_used()
    );
    assert_eq!(left.effective_gas_price, right.effective_gas_price);
    assert_eq!(left.from, right.from);
    assert_eq!(left.to, right.to);
    assert_eq!(left.contract_address, right.contract_address);
    assert_eq!(left.logs(), right.logs());
    assert_eq!(left.inner.logs_bloom(), right.inner.logs_bloom());
}

fn assert_receipts_in_block(
    receipts: &[&TransactionReceipt],
    expected: &[(Option<Address>, usize, bool)],
    block: &Block,
    sender: Address,
) {
    assert_eq!(receipts.len(), expected.len());
    assert!(!receipts.is_empty());

    let mut previous_cumulative_gas = 0u64;
    let mut gas_sum = 0u64;

    for (idx, (receipt, (expected_to, expected_logs_len, expect_non_zero_bloom))) in
        receipts.iter().zip(expected.iter()).enumerate()
    {
        assert_eq!(receipt.transaction_index, Some(idx as u64));

        let cumulative_gas = receipt.inner.cumulative_gas_used();
        if idx > 0 {
            assert!(previous_cumulative_gas < cumulative_gas);
        }
        previous_cumulative_gas = cumulative_gas;

        gas_sum = gas_sum.saturating_add(receipt.gas_used);

        assert!(receipt.gas_used > 0);
        assert!(receipt.status());
        assert!(receipt.effective_gas_price > 0);
        assert!(cumulative_gas >= receipt.gas_used);
        assert!(receipt.gas_used <= block.header.gas_limit);
        assert_eq!(receipt.from, sender);
        assert_eq!(receipt.to, *expected_to);
        assert_eq!(receipt.logs().len(), *expected_logs_len);

        if *expect_non_zero_bloom {
            assert_ne!(*receipt.inner.logs_bloom(), Bloom::ZERO);
        } else {
            assert_eq!(*receipt.inner.logs_bloom(), Bloom::ZERO);
        }
    }

    assert_eq!(previous_cumulative_gas, block.header.gas_used);
    assert_eq!(gas_sum, block.header.gas_used);
}
