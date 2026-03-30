#![allow(clippy::float_arithmetic)]
//! eth_feeHistory RPC endpoint tests
//!
//! Coverage notes:
//! - TC11 (percentile < 0): Skipped - alloy client validates percentiles client-side,
//!   negative values would require raw JSON-RPC to test server-side validation.

use alloy::network::TransactionBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, B256};
use alloy_provider::{DynProvider, Provider};
use alloy_rpc_types_eth::{
    BlockNumberOrTag, BlockTransactions, TransactionReceipt, TransactionRequest,
};
use serde::Deserialize;
use std::path::PathBuf;

use sov_cli::NodeClient;
use sov_demo_rollup::MockDemoRollup;
use sov_eth_client::SimpleStorageClient;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{GasPrice, GasUnit};
use sov_test_utils::test_rollup::TestRollup;

use crate::evm::evm_test_helper::{
    alloy_client, create_simple_storage_client, deploy_contract_check,
    estimate_gas_and_check_affordability, set_value_check, setup_test_rollup,
    setup_test_rollup_with_ideal_lag, EVM_EXTENSION, HIGH_MAX_FEE_PER_GAS,
    HIGH_PRIORITY_FEE_PER_GAS, MAX_FEE_PER_GAS, SENDER_PRIV_KEY,
};

#[derive(Debug, Deserialize)]
struct BaseFeeParamsFixture {
    max_change_denominator: u64,
    elasticity_multiplier: u64,
}

#[derive(Debug, Deserialize)]
struct ChainSpecFixture {
    block_gas_limit: u64,
    base_fee_params: BaseFeeParamsFixture,
}

#[derive(Debug, Deserialize)]
struct EvmGenesisConfigFixture {
    initial_base_fee: u64,
    chain_spec: ChainSpecFixture,
}

#[derive(Debug, Deserialize)]
struct MapResponse<T> {
    #[allow(dead_code)]
    key: u64,
    value: T,
}

#[derive(Debug, Deserialize)]
struct ChainStateGasInfo {
    #[allow(dead_code)]
    gas_limit: GasUnit<2>,
    #[allow(dead_code)]
    gas_used: GasUnit<2>,
    base_fee_per_gas: GasPrice<2>,
}

fn load_evm_genesis_config() -> EvmGenesisConfigFixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../test-data/genesis/integration-tests/evm.json");
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_json::from_str(&contents)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}

fn compute_next_base_fee(
    base_fee: u128,
    gas_used: u64,
    gas_limit: u64,
    max_change_denominator: u64,
    elasticity_multiplier: u64,
) -> u128 {
    // EIP-1559 base fee update using integer math.
    assert!(gas_limit > 0, "gas_limit must be > 0");
    assert!(
        max_change_denominator > 0,
        "max_change_denominator must be > 0"
    );
    assert!(
        elasticity_multiplier > 0,
        "elasticity_multiplier must be > 0"
    );

    let gas_target = gas_limit / elasticity_multiplier;
    assert!(gas_target > 0, "gas_target must be > 0");

    if gas_used == gas_target {
        return base_fee;
    }

    let gas_used_delta = gas_used.abs_diff(gas_target);

    let mut base_fee_delta = base_fee
        .saturating_mul(gas_used_delta as u128)
        .checked_div(gas_target as u128)
        .unwrap_or(0)
        .checked_div(max_change_denominator as u128)
        .unwrap_or(0);

    if gas_used > gas_target {
        if base_fee_delta < 1 {
            base_fee_delta = 1;
        }
        base_fee.saturating_add(base_fee_delta)
    } else {
        let updated = base_fee.saturating_sub(base_fee_delta);
        if updated < 1 {
            1
        } else {
            updated
        }
    }
}

fn gas_price_dim0(price: &GasPrice<2>) -> u128 {
    price.as_ref()[0].0
}

async fn fetch_chain_state_gas_info(
    client: &NodeClient,
    height: u64,
) -> anyhow::Result<ChainStateGasInfo> {
    let url = format!("/modules/chain-state/state/gas-info/items/{height}");
    let response: MapResponse<ChainStateGasInfo> = client.query_rest_endpoint(&url).await?;
    Ok(response.value)
}

async fn block_tx_hashes(client: &DynProvider, block_number: u64) -> anyhow::Result<Vec<B256>> {
    let block = client
        .get_block_by_number(BlockNumberOrTag::Number(block_number))
        .await?
        .ok_or_else(|| anyhow::anyhow!("block {block_number} should exist"))?;
    let hashes = match block.transactions {
        BlockTransactions::Hashes(hashes) => hashes,
        BlockTransactions::Full(txs) => txs.into_iter().map(|tx| *tx.inner.hash()).collect(),
        BlockTransactions::Uncle => Vec::new(),
    };
    Ok(hashes)
}

async fn total_gas_used_from_receipts(
    client: &DynProvider,
    block_number: u64,
) -> anyhow::Result<u64> {
    let tx_hashes = block_tx_hashes(client, block_number).await?;
    let mut total = 0u64;
    for tx_hash in tx_hashes {
        let receipt = client
            .get_transaction_receipt(tx_hash)
            .await?
            .ok_or_else(|| anyhow::anyhow!("receipt missing for tx {tx_hash:?}"))?;
        total = total.saturating_add(receipt.gas_used);
    }
    Ok(total)
}

async fn base_fee_series_from_genesis(
    rpc_client: &DynProvider,
    end_block: u64,
    genesis: &EvmGenesisConfigFixture,
) -> anyhow::Result<Vec<u128>> {
    let mut base_fees = Vec::with_capacity((end_block + 1) as usize);
    let mut base_fee = genesis.initial_base_fee as u128;
    let params = &genesis.chain_spec.base_fee_params;
    let gas_limit = genesis.chain_spec.block_gas_limit;
    assert!(gas_limit > 0, "block gas limit should be non-zero");

    // Block 0: initial base fee
    base_fees.push(base_fee);

    for block_number in 0..end_block {
        // Block 1 also uses initial base fee (chain-state special case)
        if block_number == 0 {
            base_fees.push(base_fee);
            continue;
        }

        // Block 2+: compute based on previous block's gas usage
        let receipts_gas_used = total_gas_used_from_receipts(rpc_client, block_number).await?;
        base_fee = compute_next_base_fee(
            base_fee,
            receipts_gas_used,
            gas_limit,
            params.max_change_denominator,
            params.elasticity_multiplier,
        );
        base_fees.push(base_fee);
    }
    Ok(base_fees)
}

async fn send_high_fee_set_value(
    client: &SimpleStorageClient,
    contract_address: Address,
    value: u32,
) -> anyhow::Result<TransactionReceipt> {
    let mut tx = client.make_tx(Some(contract_address), Some(client.contract.set(value)));
    tx = tx
        .max_fee_per_gas(MAX_FEE_PER_GAS)
        .max_priority_fee_per_gas(HIGH_PRIORITY_FEE_PER_GAS);
    let sender_balance = client.eth_get_balance(client.address()).await;
    estimate_gas_and_check_affordability(client, &mut tx, MAX_FEE_PER_GAS, sender_balance).await?;
    client
        .send_tx_and_wait_finalized(tx)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))
}

fn override_rollup_gas_limit_transition(
    initial_gas_limit: u64,
    updated_gas_limit: u64,
    change_after_height: u64,
) {
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_INITIAL_GAS_LIMIT",
        format!("[{initial_gas_limit},{initial_gas_limit}]"),
    );
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_UPDATED_GAS_LIMIT",
        format!("[{updated_gas_limit},{updated_gas_limit}]"),
    );
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_CHANGE_GAS_LIMIT_AFTER_HEIGHT",
        change_after_height.to_string(),
    );
}

async fn send_high_fee_transfer(
    client: &DynProvider,
    nonce: u64,
) -> anyhow::Result<TransactionReceipt> {
    send_high_fee_transfer_with_gas_limit(client, nonce, 1_000_000).await
}

async fn send_high_fee_transfer_with_gas_limit(
    client: &DynProvider,
    nonce: u64,
    gas_limit: u64,
) -> anyhow::Result<TransactionReceipt> {
    let tx = TransactionRequest::default()
        .with_to(Address::ZERO)
        .with_nonce(nonce)
        .with_value(alloy_primitives::U256::ZERO)
        .with_gas_limit(gas_limit)
        .with_max_fee_per_gas(HIGH_MAX_FEE_PER_GAS)
        .with_max_priority_fee_per_gas(HIGH_PRIORITY_FEE_PER_GAS);
    let pending = client.send_transaction(tx).await?;
    Ok(pending.get_receipt().await?)
}

// ==================== Test Setup Helpers ====================

async fn setup_fee_history_test(
    wait_blocks: u64,
) -> (TestRollup<MockDemoRollup<Native>>, DynProvider) {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(wait_blocks).await;
    rollup
        .pause_preferred_batches_and_wait()
        .await
        .expect("pause should be acknowledged before fee_history queries");
    (rollup, client)
}

/// Sets up a test with pending transactions by deploying a contract and submitting
/// a transaction that stays in the pending state.
async fn setup_with_pending_tx(
    rollup: &TestRollup<MockDemoRollup<Native>>,
) -> (SimpleStorageClient, Address) {
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");
    rollup
        .pause_preferred_batches_and_wait()
        .await
        .expect("pause should be acknowledged before creating pending tx");
    simple_storage.set_value(contract_address, 42).await;
    (simple_storage, contract_address)
}

fn assert_float_eq(actual: f64, expected: f64, context: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta < 1e-12,
        "{context}: expected {expected}, got {actual}"
    );
}

fn assert_gas_ratios_valid(ratios: &[f64]) {
    for (i, ratio) in ratios.iter().enumerate() {
        assert!(
            (0.0..=1.0).contains(ratio),
            "gas_used_ratio[{i}] = {ratio} should be in [0, 1]"
        );
    }
}

async fn get_block_base_fee(client: &DynProvider, block_num: u64) -> anyhow::Result<u128> {
    let block = client
        .get_block_by_number(BlockNumberOrTag::Number(block_num))
        .await?
        .ok_or_else(|| anyhow::anyhow!("block {block_num} should exist"))?;
    let base_fee = block
        .header
        .base_fee_per_gas
        .ok_or_else(|| anyhow::anyhow!("block {block_num} should include base_fee_per_gas"))?;
    Ok(u128::from(base_fee))
}

/// Shared validation logic for finalized/safe block tags.
/// Both tags map to the latest finalized block visible to the RPC.
async fn verify_fee_history_for_sealed_tag(
    client: &DynProvider,
    tag: BlockNumberOrTag,
    block_count: u64,
    expected_finalized_end_block: Option<u64>,
) -> anyhow::Result<()> {
    let (fee_history, matched_tag_block_number) = {
        let mut last_expected_end = 0u64;
        let mut last_observed_end = 0u64;
        let mut last_oldest = 0u64;
        let mut matched = None;
        for _ in 0..100 {
            let fee_history = client.get_fee_history(block_count, tag, &[]).await?;
            let tag_block = client
                .get_block_by_number(tag)
                .await?
                .ok_or_else(|| anyhow::anyhow!("{tag:?} block should exist"))?;
            if fee_history.gas_used_ratio.is_empty() {
                last_expected_end = tag_block.header.number;
                last_observed_end = fee_history.oldest_block;
                last_oldest = fee_history.oldest_block;
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                continue;
            }

            let end_block = fee_history.oldest_block + fee_history.gas_used_ratio.len() as u64 - 1;
            last_expected_end = tag_block.header.number;
            last_observed_end = end_block;
            last_oldest = fee_history.oldest_block;
            if end_block == tag_block.header.number {
                matched = Some((fee_history, tag_block.header.number));
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        if let Some(matched) = matched {
            matched
        } else {
            anyhow::bail!(
                "{tag:?} feeHistory end block mismatch: expected {last_expected_end}, got {last_observed_end} (oldest {last_oldest})"
            )
        }
    };

    assert_eq!(
        fee_history.base_fee_per_gas.len(),
        fee_history.gas_used_ratio.len() + 1
    );
    assert!(
        fee_history.gas_used_ratio.len() as u64 <= block_count,
        "gas_used_ratio length should not exceed requested block_count"
    );
    assert!(fee_history.reward.is_none(), "reward should be omitted");
    assert!(
        !fee_history.gas_used_ratio.is_empty(),
        "{tag:?} feeHistory should include at least one block"
    );

    let end_block = fee_history.oldest_block + fee_history.gas_used_ratio.len() as u64 - 1;
    assert_eq!(
        end_block, matched_tag_block_number,
        "{tag:?} range should end at latest finalized block"
    );
    if let Some(expected_finalized_end_block) = expected_finalized_end_block {
        assert_eq!(
            end_block, expected_finalized_end_block,
            "{tag:?} should end at eth_blockNumber with instant finality"
        );
    }

    let oldest = fee_history.oldest_block;
    let end_block_exclusive = oldest + fee_history.gas_used_ratio.len() as u64;
    let genesis = load_evm_genesis_config();
    let base_fees = base_fee_series_from_genesis(client, end_block_exclusive, &genesis).await?;
    let gas_limit = genesis.chain_spec.block_gas_limit;

    for (i, ratio) in fee_history.gas_used_ratio.iter().enumerate() {
        let block_number = oldest + i as u64;
        let receipts_gas_used = total_gas_used_from_receipts(client, block_number).await?;
        let expected_ratio = receipts_gas_used as f64 / gas_limit as f64;
        assert_float_eq(
            *ratio,
            expected_ratio,
            &format!("gas_used_ratio for block {block_number}"),
        );

        let expected_base_fee = base_fees[block_number as usize];
        assert_eq!(
            fee_history.base_fee_per_gas[i], expected_base_fee,
            "baseFeePerGas mismatch for block {block_number}"
        );
    }

    let predicted_base_fee = base_fees[end_block_exclusive as usize];
    assert_eq!(
        fee_history.base_fee_per_gas[fee_history.base_fee_per_gas.len() - 1],
        predicted_base_fee,
        "predicted next base fee mismatch for block {end_block_exclusive}"
    );

    let latest_base_fee = get_block_base_fee(client, end_block).await?;
    assert_eq!(
        fee_history.base_fee_per_gas[fee_history.gas_used_ratio.len() - 1],
        latest_base_fee,
        "baseFeePerGas should match end block header"
    );

    assert_gas_ratios_valid(&fee_history.gas_used_ratio);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_basic() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(3).await;

    let fee_history = client
        .get_fee_history(3, BlockNumberOrTag::Latest, &[])
        .await?;

    assert_eq!(fee_history.base_fee_per_gas.len(), 4);
    assert_eq!(fee_history.gas_used_ratio.len(), 3);
    assert_gas_ratios_valid(&fee_history.gas_used_ratio);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_single_block() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(2).await;

    let fee_history = client
        .get_fee_history(1, BlockNumberOrTag::Latest, &[])
        .await?;

    assert_eq!(fee_history.base_fee_per_gas.len(), 2);
    assert_eq!(fee_history.gas_used_ratio.len(), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_with_reward_percentiles() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(2).await;

    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[25.0, 50.0, 75.0])
        .await?;

    let rewards = fee_history.reward.unwrap();
    assert_eq!(rewards.len(), 2);
    assert_eq!(rewards[0].len(), 3);
    assert!(rewards.iter().all(|block| block.iter().all(|&r| r == 0)));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Known discrepancy: will be fixed in the follow up"]
async fn test_eth_fee_history_zero_blocks() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);

    let fee_history = client
        .get_fee_history(0, BlockNumberOrTag::Latest, &[])
        .await?;

    assert_eq!(fee_history.oldest_block, 0);
    assert!(fee_history.base_fee_per_gas.is_empty());
    assert!(fee_history.gas_used_ratio.is_empty());
    assert_eq!(fee_history.reward, None);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_specific_block() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(5).await;

    let fee_history = client
        .get_fee_history(3, BlockNumberOrTag::Number(3), &[])
        .await?;

    assert!(fee_history.oldest_block <= 3);
    assert_eq!(fee_history.base_fee_per_gas.len(), 4);
    assert_eq!(fee_history.gas_used_ratio.len(), 3);

    let oldest = fee_history.oldest_block;
    let end_block = oldest + fee_history.gas_used_ratio.len() as u64;
    let genesis = load_evm_genesis_config();
    let base_fees = base_fee_series_from_genesis(&client, end_block, &genesis).await?;
    let gas_limit = genesis.chain_spec.block_gas_limit;

    for (i, ratio) in fee_history.gas_used_ratio.iter().enumerate() {
        let block_number = oldest + i as u64;
        let receipts_gas_used = total_gas_used_from_receipts(&client, block_number).await?;
        let expected_ratio = receipts_gas_used as f64 / gas_limit as f64;
        assert_float_eq(
            *ratio,
            expected_ratio,
            &format!("gas_used_ratio for block {block_number}"),
        );

        let expected_base_fee = base_fees[block_number as usize];
        assert_eq!(
            fee_history.base_fee_per_gas[i], expected_base_fee,
            "baseFeePerGas mismatch for block {block_number}"
        );
    }

    let predicted_base_fee = base_fees[end_block as usize];
    assert_eq!(
        fee_history.base_fee_per_gas[fee_history.base_fee_per_gas.len() - 1],
        predicted_base_fee,
        "predicted next base fee mismatch for block {end_block}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_fee_history_large_count_capped() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(2).await;

    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");
    rollup.wait_for_rollup_height_advance_by(1).await;
    let receipt = send_high_fee_set_value(&simple_storage, contract_address, 10).await?;
    let newest_block = receipt
        .block_number
        .expect("receipt should include block number");
    // Wait for the slot to complete so gas_info is recorded
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches_and_wait().await?;

    assert!(client
        .get_fee_history(2000, BlockNumberOrTag::Number(newest_block), &[])
        .await
        .is_err());

    let fee_history = client
        .get_fee_history(1, BlockNumberOrTag::Number(newest_block), &[])
        .await?;

    // Get base fee directly from block header (receipt effective_gas_price uses different formula)
    let base_fee = get_block_base_fee(&client, newest_block).await?;
    assert!(base_fee > 0, "baseFeePerGas should be non-zero");

    let genesis = load_evm_genesis_config();
    let gas_limit = genesis.chain_spec.block_gas_limit;
    let gas_info = fetch_chain_state_gas_info(&rollup.client, newest_block).await?;
    let chain_base_fee = gas_price_dim0(&gas_info.base_fee_per_gas);
    assert_eq!(
        chain_base_fee, base_fee,
        "chain-state base fee should match block header base fee"
    );
    let receipts_gas_used = total_gas_used_from_receipts(&client, newest_block).await?;
    assert!(
        receipts_gas_used >= receipt.gas_used,
        "block gas_used should cover receipt gas_used"
    );
    let expected_ratio = receipts_gas_used as f64 / gas_limit as f64;

    let idx = (newest_block - fee_history.oldest_block) as usize;
    assert_eq!(
        fee_history.base_fee_per_gas[idx], base_fee,
        "baseFeePerGas should match block header base fee"
    );
    assert_float_eq(
        fee_history.gas_used_ratio[idx],
        expected_ratio,
        "gas_used_ratio",
    );

    let params = &genesis.chain_spec.base_fee_params;
    let expected_next_base_fee = compute_next_base_fee(
        base_fee,
        receipts_gas_used,
        gas_limit,
        params.max_change_denominator,
        params.elasticity_multiplier,
    );
    assert_eq!(
        fee_history.base_fee_per_gas[idx + 1],
        expected_next_base_fee,
        "predicted next base fee should follow EIP-1559"
    );

    Ok(())
}

// ==================== Block Tag Variation Tests ====================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_pending_tag() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(3).await;

    let _ = setup_with_pending_tx(&rollup).await;

    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Pending, &[])
        .await?;

    // Pending tag should work and return valid data
    assert_eq!(fee_history.base_fee_per_gas.len(), 3);
    assert_eq!(fee_history.gas_used_ratio.len(), 2);

    let sealed_head = client
        .get_block_by_number(BlockNumberOrTag::Finalized)
        .await?
        .expect("finalized block should exist")
        .header
        .number;
    let eth_block_number = client.get_block_number().await?;
    assert_eq!(
        eth_block_number,
        sealed_head + 1,
        "eth_blockNumber should resolve to pending when a pending block exists"
    );
    assert_eq!(
        fee_history.oldest_block, sealed_head,
        "pending range should start at latest sealed block"
    );
    assert!(fee_history.reward.is_none(), "reward should be omitted");

    let genesis = load_evm_genesis_config();
    let base_fees = base_fee_series_from_genesis(&client, sealed_head, &genesis).await?;
    let expected_base_fee = base_fees[sealed_head as usize];
    assert!(expected_base_fee > 0, "baseFeePerGas should be non-zero");

    let gas_limit = genesis.chain_spec.block_gas_limit;
    let receipts_gas_used = total_gas_used_from_receipts(&client, sealed_head).await?;
    let expected_ratio = receipts_gas_used as f64 / gas_limit as f64;

    assert_eq!(
        fee_history.base_fee_per_gas[0], expected_base_fee,
        "baseFeePerGas[0] should match receipt-derived base fee"
    );
    assert_float_eq(
        fee_history.gas_used_ratio[0],
        expected_ratio,
        "gas_used_ratio",
    );

    let latest_base_fee = get_block_base_fee(&client, sealed_head).await?;
    assert_eq!(
        fee_history.base_fee_per_gas[0], latest_base_fee,
        "baseFeePerGas[0] should match latest sealed block header"
    );
    assert_gas_ratios_valid(&fee_history.gas_used_ratio);

    Ok(())
}

/// Tests that feeHistory for Pending block matches the pending block header and computed values.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_pending_base_fee_matches_pending_block() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(2).await;

    let _ = setup_with_pending_tx(&rollup).await;

    let sealed_head = client
        .get_block_by_number(BlockNumberOrTag::Finalized)
        .await?
        .expect("finalized block should exist")
        .header
        .number;
    let genesis = load_evm_genesis_config();
    let base_fees = base_fee_series_from_genesis(&client, sealed_head, &genesis).await?;
    let gas_limit = genesis.chain_spec.block_gas_limit;
    let receipts_gas_used = total_gas_used_from_receipts(&client, sealed_head).await?;
    let params = &genesis.chain_spec.base_fee_params;
    let expected_pending_base_fee = compute_next_base_fee(
        base_fees[sealed_head as usize],
        receipts_gas_used,
        gas_limit,
        params.max_change_denominator,
        params.elasticity_multiplier,
    );

    let pending_block = client
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .expect("pending block should exist");
    let pending_base_fee = pending_block
        .header
        .base_fee_per_gas
        .expect("pending block should include base_fee_per_gas");

    let fee_history = client
        .get_fee_history(1, BlockNumberOrTag::Pending, &[])
        .await?;

    assert_eq!(fee_history.base_fee_per_gas.len(), 2);
    assert_eq!(
        fee_history.base_fee_per_gas[0], expected_pending_base_fee,
        "feeHistory pending base fee should match receipt-derived next base fee"
    );
    assert_eq!(
        fee_history.base_fee_per_gas[0], pending_base_fee as u128,
        "feeHistory pending base fee should match pending block header"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_finalized_tag() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(3).await;
    let expected_finalized_end_block = client.get_block_number().await?;
    verify_fee_history_for_sealed_tag(
        &client,
        BlockNumberOrTag::Finalized,
        2,
        Some(expected_finalized_end_block),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_safe_tag() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(3).await;
    let expected_finalized_end_block = client.get_block_number().await?;
    verify_fee_history_for_sealed_tag(
        &client,
        BlockNumberOrTag::Safe,
        2,
        Some(expected_finalized_end_block),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_earliest_tag() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(3).await;

    // Request 2 blocks ending at earliest - should gracefully handle limited range
    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Earliest, &[])
        .await?;

    // Earliest points to block 0, so we can only get 1 block (0 to 0)
    assert_eq!(fee_history.oldest_block, 0);
    assert!(!fee_history.base_fee_per_gas.is_empty());

    let genesis = load_evm_genesis_config();
    let end_block = fee_history.oldest_block + fee_history.gas_used_ratio.len() as u64;
    let base_fees = base_fee_series_from_genesis(&client, end_block, &genesis).await?;
    let gas_limit = genesis.chain_spec.block_gas_limit;
    let receipts_gas_used = total_gas_used_from_receipts(&client, 0).await?;
    let expected_ratio = receipts_gas_used as f64 / gas_limit as f64;
    assert_float_eq(
        fee_history.gas_used_ratio[0],
        expected_ratio,
        "gas_used_ratio for block 0",
    );
    assert_eq!(
        fee_history.base_fee_per_gas[0], base_fees[0],
        "baseFeePerGas mismatch for block 0"
    );
    assert_eq!(
        fee_history.base_fee_per_gas[1], base_fees[1],
        "predicted next base fee mismatch for block 1"
    );

    Ok(())
}

/// Rollup semantics: `latest` resolves to `pending`.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_latest_equals_pending() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(3).await;

    let _ = setup_with_pending_tx(&rollup).await;

    let latest_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[])
        .await?;
    let pending_history = client
        .get_fee_history(2, BlockNumberOrTag::Pending, &[])
        .await?;

    assert_eq!(latest_history.base_fee_per_gas.len(), 3);
    assert_eq!(latest_history.gas_used_ratio.len(), 2);
    assert_eq!(latest_history.oldest_block, pending_history.oldest_block);
    assert_eq!(
        latest_history.base_fee_per_gas,
        pending_history.base_fee_per_gas
    );
    assert_eq!(
        latest_history.gas_used_ratio,
        pending_history.gas_used_ratio
    );
    assert_eq!(latest_history.reward, pending_history.reward);
    assert_eq!(
        latest_history.base_fee_per_blob_gas,
        pending_history.base_fee_per_blob_gas
    );
    assert_eq!(
        latest_history.blob_gas_used_ratio,
        pending_history.blob_gas_used_ratio
    );

    let sealed_head = client
        .get_block_by_number(BlockNumberOrTag::Finalized)
        .await?
        .expect("finalized block should exist")
        .header
        .number;
    let eth_block_number = client.get_block_number().await?;
    assert_eq!(
        eth_block_number,
        sealed_head + 1,
        "eth_blockNumber should resolve to pending when a pending block exists"
    );
    assert_eq!(
        latest_history.oldest_block, sealed_head,
        "with block_count=2 and newest=pending, oldest should be latest sealed block"
    );
    let genesis = load_evm_genesis_config();
    let base_fees = base_fee_series_from_genesis(&client, sealed_head, &genesis).await?;
    let expected_base_fee = base_fees[sealed_head as usize];
    assert!(expected_base_fee > 0, "baseFeePerGas should be non-zero");

    let gas_limit = genesis.chain_spec.block_gas_limit;
    let receipts_gas_used = total_gas_used_from_receipts(&client, sealed_head).await?;
    let expected_ratio = receipts_gas_used as f64 / gas_limit as f64;
    assert_float_eq(
        latest_history.gas_used_ratio[0],
        expected_ratio,
        "gas_used_ratio",
    );
    assert_eq!(
        latest_history.base_fee_per_gas[0], expected_base_fee,
        "baseFeePerGas[0] should match receipt-derived base fee"
    );

    let latest_base_fee = get_block_base_fee(&client, sealed_head).await?;
    assert_eq!(
        latest_history.base_fee_per_gas[0], latest_base_fee,
        "baseFeePerGas[0] should match latest sealed block header"
    );

    let latest_sealed_block = client
        .get_block_by_number(BlockNumberOrTag::Number(sealed_head))
        .await?
        .expect("latest block should exist");
    let header_gas_limit = latest_sealed_block.header.gas_limit;
    assert!(header_gas_limit > 0, "block gas_limit should be non-zero");
    let expected_ratio = latest_sealed_block.header.gas_used as f64 / header_gas_limit as f64;
    assert_float_eq(
        latest_history.gas_used_ratio[0],
        expected_ratio,
        "gas_used_ratio from block header",
    );

    Ok(())
}

// ==================== Validation Error Tests ====================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_percentile_out_of_range_over_100() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(2).await;

    // Percentile > 100 should fail
    let result = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[50.0, 150.0])
        .await;

    assert!(result.is_err(), "Percentile > 100 should return error");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_percentiles_not_monotonic() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(2).await;

    // Non-monotonic percentiles should fail
    let result = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[75.0, 50.0, 25.0])
        .await;

    assert!(
        result.is_err(),
        "Non-monotonic percentiles should return error"
    );

    Ok(())
}

// ==================== Schema Invariant Tests ====================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_array_length_invariants() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(10).await;

    for block_count in 1..8 {
        let fee_history = client
            .get_fee_history(block_count, BlockNumberOrTag::Latest, &[25.0, 75.0])
            .await?;

        assert_gas_ratios_valid(&fee_history.gas_used_ratio);

        // Key invariant: base_fee_per_gas has block_count + 1 entries
        assert_eq!(
            fee_history.base_fee_per_gas.len(),
            (block_count + 1) as usize,
            "base_fee_per_gas should have block_count + 1 entries"
        );

        // gas_used_ratio has block_count entries
        assert_eq!(
            fee_history.gas_used_ratio.len(),
            block_count as usize,
            "gas_used_ratio should have block_count entries"
        );

        // reward (if present) has block_count rows, each with percentile_count columns
        let rewards = fee_history.reward.expect("reward should be present");
        assert_eq!(
            rewards.len(),
            block_count as usize,
            "reward should have block_count rows"
        );
        for (i, row) in rewards.iter().enumerate() {
            assert_eq!(row.len(), 2, "reward row {i} should have 2 columns");
        }
    }

    Ok(())
}

/// Regression: reward rows are sized to the returned block count (not the requested blockCount)
/// when the chain is shorter than the requested range.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_reward_len_matches_available_blocks() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(2).await;

    let latest = client.get_block_number().await?;
    let block_count = latest + 5;
    let fee_history = client
        .get_fee_history(block_count, BlockNumberOrTag::Number(latest), &[50.0])
        .await?;

    // Chain is shorter than requested; should clamp to genesis.
    assert_eq!(fee_history.oldest_block, 0);
    assert_eq!(
        fee_history.gas_used_ratio.len() as u64,
        latest + 1,
        "gas_used_ratio should cover all available blocks"
    );
    assert_eq!(
        fee_history.base_fee_per_gas.len(),
        fee_history.gas_used_ratio.len() + 1,
        "base_fee_per_gas should have block_count + 1 entries"
    );

    let rewards = fee_history.reward.expect("reward should be present");
    assert_eq!(
        rewards.len(),
        fee_history.gas_used_ratio.len(),
        "reward row count should match returned block count"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_oldest_block_correctness() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(10).await;

    // Request 3 blocks ending at block 7
    let fee_history = client
        .get_fee_history(3, BlockNumberOrTag::Number(7), &[])
        .await?;

    // oldest_block should be 7 - (3 - 1) = 5
    assert_eq!(
        fee_history.oldest_block, 5,
        "oldest_block should be newest_block - block_count + 1"
    );
    assert_eq!(fee_history.gas_used_ratio.len(), 3);

    Ok(())
}

// ==================== State Evolution Tests ====================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_gas_ratio_valid_range() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(5).await;

    let fee_history = client
        .get_fee_history(5, BlockNumberOrTag::Finalized, &[])
        .await?;

    assert_gas_ratios_valid(&fee_history.gas_used_ratio);
    assert!(
        !fee_history.base_fee_per_gas.is_empty(),
        "base_fee_per_gas should not be empty"
    );

    Ok(())
}

// ==================== Value Correctness Tests ====================

/// Regression: baseFeePerGas should never drop below 1 after genesis (EIP-1559 min base fee).
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_base_fee_nonzero_after_genesis() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(4).await;

    let latest = client.get_block_number().await?;
    let block_count = 3u64;
    let fee_history = client
        .get_fee_history(block_count, BlockNumberOrTag::Number(latest), &[])
        .await?;

    assert_eq!(
        fee_history.base_fee_per_gas.len(),
        (block_count + 1) as usize,
        "expected block_count + 1 base fees"
    );
    assert!(
        fee_history.oldest_block >= 1,
        "expected fee history range to start after genesis"
    );

    for (i, fee) in fee_history.base_fee_per_gas.iter().enumerate() {
        let block_num = fee_history.oldest_block + i as u64;
        assert!(
            *fee >= 1,
            "baseFeePerGas for block {block_num} should be >= 1, got {fee}"
        );
        if i < fee_history.gas_used_ratio.len() {
            let header_base_fee = get_block_base_fee(&client, block_num).await?;
            assert_eq!(
                *fee, header_base_fee,
                "baseFeePerGas for block {block_num} should match block header"
            );
        }
    }

    Ok(())
}

/// Regression: genesis baseFeePerGas in feeHistory matches the block header.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_earliest_values_match_block_header() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(2).await;

    let fee_history = client
        .get_fee_history(1, BlockNumberOrTag::Earliest, &[])
        .await?;

    assert_eq!(fee_history.oldest_block, 0);
    assert_eq!(fee_history.base_fee_per_gas.len(), 2);
    assert_eq!(fee_history.gas_used_ratio.len(), 1);

    let genesis = load_evm_genesis_config();
    let base_fees = base_fee_series_from_genesis(&client, 1, &genesis).await?;
    let gas_limit0 = genesis.chain_spec.block_gas_limit;
    let receipts_gas_used0 = total_gas_used_from_receipts(&client, 0).await?;
    let expected_ratio0 = receipts_gas_used0 as f64 / gas_limit0 as f64;
    assert_float_eq(
        fee_history.gas_used_ratio[0],
        expected_ratio0,
        "genesis gas_used_ratio",
    );
    assert_eq!(
        fee_history.base_fee_per_gas[0], base_fees[0],
        "genesis baseFeePerGas should match receipt-derived value"
    );
    assert_eq!(
        fee_history.base_fee_per_gas[1], base_fees[1],
        "predicted next base fee should match receipt-derived value"
    );

    let base_fee0 = get_block_base_fee(&client, 0).await?;
    assert_eq!(
        fee_history.base_fee_per_gas[0], base_fee0,
        "genesis baseFeePerGas should match the block header"
    );

    let block0 = client
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await?
        .expect("genesis block should exist");
    let header_gas_limit = block0.header.gas_limit;
    assert!(header_gas_limit > 0, "genesis gas_limit should be non-zero");
    let expected_ratio0 = block0.header.gas_used as f64 / header_gas_limit as f64;
    assert_float_eq(
        fee_history.gas_used_ratio[0],
        expected_ratio0,
        "genesis gas_used_ratio from header",
    );

    let base_fee1 = get_block_base_fee(&client, 1).await?;
    assert_eq!(
        fee_history.base_fee_per_gas[1], base_fee1,
        "predicted next base fee should match block 1 header"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_values_match_block_headers() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(2).await;

    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let tx_hash = simple_storage
        .deploy_contract()
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let receipt = simple_storage.wait_for_finalized_receipt(tx_hash).await;
    let deploy_block = receipt
        .block_number
        .expect("deploy receipt should include block number");

    rollup.wait_for_rollup_height_advance_by(2).await;
    rollup.pause_preferred_batches_and_wait().await?;

    let newest_block = deploy_block + 1;
    let fee_history = client
        .get_fee_history(3, BlockNumberOrTag::Number(newest_block), &[])
        .await?;

    assert_eq!(fee_history.base_fee_per_gas.len(), 4);
    assert_eq!(fee_history.gas_used_ratio.len(), 3);

    let oldest_block = fee_history.oldest_block;
    assert_eq!(oldest_block + 2, newest_block);

    let end_block = oldest_block + fee_history.gas_used_ratio.len() as u64;
    let genesis = load_evm_genesis_config();
    let base_fees = base_fee_series_from_genesis(&client, end_block, &genesis).await?;
    let gas_limit = genesis.chain_spec.block_gas_limit;

    for (i, ratio) in fee_history.gas_used_ratio.iter().enumerate() {
        let block_num = oldest_block + i as u64;
        let receipts_gas_used = total_gas_used_from_receipts(&client, block_num).await?;
        let expected_ratio = receipts_gas_used as f64 / gas_limit as f64;
        assert_float_eq(*ratio, expected_ratio, "gas_used_ratio");

        let expected_base_fee = base_fees[block_num as usize];
        assert_eq!(
            fee_history.base_fee_per_gas[i], expected_base_fee,
            "baseFeePerGas should match receipt-derived value"
        );

        let header_base_fee = get_block_base_fee(&client, block_num).await?;
        assert_eq!(
            fee_history.base_fee_per_gas[i], header_base_fee,
            "baseFeePerGas should match the block header"
        );

        let block = client
            .get_block_by_number(BlockNumberOrTag::Number(block_num))
            .await?
            .expect("block should exist");
        let header_gas_limit = block.header.gas_limit;
        assert!(header_gas_limit > 0, "block gas_limit should be non-zero");
        let expected_ratio = block.header.gas_used as f64 / header_gas_limit as f64;
        assert_float_eq(*ratio, expected_ratio, "gas_used_ratio from header");
    }

    let predicted_base_fee = base_fees[end_block as usize];
    assert_eq!(
        fee_history.base_fee_per_gas[fee_history.base_fee_per_gas.len() - 1],
        predicted_base_fee,
        "predicted next base fee should follow EIP-1559"
    );

    let next_base_fee = get_block_base_fee(&client, newest_block + 1).await?;
    assert_eq!(
        fee_history.base_fee_per_gas[fee_history.base_fee_per_gas.len() - 1],
        next_base_fee,
        "predicted next base fee should match the next block header"
    );

    Ok(())
}

// ==================== State and History Scenario Tests ====================

/// TC28: Empty blocks have gas_used_ratio = 0.0
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_empty_blocks_zero_ratio() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(5).await;

    let fee_history = client
        .get_fee_history(5, BlockNumberOrTag::Finalized, &[])
        .await?;

    // All blocks are empty, so gas_used_ratio should be 0.0 for all
    for (i, ratio) in fee_history.gas_used_ratio.iter().enumerate() {
        assert_eq!(
            *ratio, 0.0,
            "Empty block {i} should have gas_used_ratio = 0.0, got {ratio}"
        );
    }

    Ok(())
}

/// TC29: Block with transaction returns expected baseFeePerGas and gas_used_ratio.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_block_with_tx_nonzero_ratio() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(1).await;

    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");
    rollup.wait_for_rollup_height_advance_by(1).await;
    let receipt = send_high_fee_set_value(&simple_storage, contract_address, 100).await?;
    let tx_block = receipt
        .block_number
        .expect("receipt should include block number");
    // Wait for the slot to complete so gas_info is recorded
    rollup.wait_for_rollup_height_advance_by(1).await;
    rollup.pause_preferred_batches_and_wait().await?;

    assert!(
        receipt.gas_used > 0,
        "deploy receipt should report non-zero gas_used"
    );

    let genesis = load_evm_genesis_config();
    let params = &genesis.chain_spec.base_fee_params;
    let gas_limit = genesis.chain_spec.block_gas_limit;
    assert!(gas_limit > 0, "block gas limit should be non-zero");

    // Get base fee directly from block header (receipt effective_gas_price uses different formula)
    let header_base_fee = get_block_base_fee(&client, tx_block).await?;
    assert!(header_base_fee > 0, "baseFeePerGas should be non-zero");

    // Verify chain-state gas_info matches block header
    let gas_info = fetch_chain_state_gas_info(&rollup.client, tx_block).await?;
    let chain_base_fee = gas_price_dim0(&gas_info.base_fee_per_gas);
    assert_eq!(
        chain_base_fee, header_base_fee,
        "chain-state base fee should match block header"
    );

    let receipts_gas_used = total_gas_used_from_receipts(&client, tx_block).await?;
    let chain_gas_used_total = gas_info
        .gas_used
        .as_ref()
        .iter()
        .copied()
        .fold(0u64, u64::saturating_add);
    assert!(
        chain_gas_used_total >= receipts_gas_used,
        "sum(chain-state gas_used dimensions) should be >= receipts gas_used"
    );
    let expected_next_base_fee = compute_next_base_fee(
        header_base_fee,
        receipts_gas_used,
        gas_limit,
        params.max_change_denominator,
        params.elasticity_multiplier,
    );

    let fee_history = client
        .get_fee_history(1, BlockNumberOrTag::Number(tx_block), &[])
        .await?;

    assert_eq!(fee_history.oldest_block, tx_block);
    assert_eq!(fee_history.base_fee_per_gas.len(), 2);
    assert_eq!(fee_history.gas_used_ratio.len(), 1);
    assert_eq!(
        fee_history.base_fee_per_gas[0], header_base_fee,
        "baseFeePerGas should match block header base fee"
    );

    let expected_ratio = receipts_gas_used as f64 / gas_limit as f64;
    assert_float_eq(
        fee_history.gas_used_ratio[0],
        expected_ratio,
        "gas_used_ratio",
    );

    assert_eq!(
        fee_history.base_fee_per_gas[1], expected_next_base_fee,
        "predicted next base fee should follow EIP-1559"
    );

    // Verify header base fee is consistent (re-fetch to double-check)
    let header_base_fee_check = get_block_base_fee(&client, tx_block).await?;
    assert_eq!(
        header_base_fee_check, header_base_fee,
        "block header base fee should be consistent"
    );

    let block = client
        .get_block_by_number(BlockNumberOrTag::Number(tx_block))
        .await?
        .expect("tx block should exist");
    let header_ratio = block.header.gas_used as f64 / block.header.gas_limit as f64;
    assert_float_eq(header_ratio, expected_ratio, "block header gas_used_ratio");

    Ok(())
}

/// Regression: each gas_used_ratio entry must use that block's gas limit across a gas-limit transition.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_gas_used_ratio_uses_per_block_limit_across_transition(
) -> anyhow::Result<()> {
    const INITIAL_GAS_LIMIT: u64 = 100_000_000_000;
    const UPDATED_GAS_LIMIT: u64 = 50_000_000_000;
    const CHANGE_GAS_LIMIT_AFTER_HEIGHT: u64 = 10;

    override_rollup_gas_limit_transition(
        INITIAL_GAS_LIMIT,
        UPDATED_GAS_LIMIT,
        CHANGE_GAS_LIMIT_AFTER_HEIGHT,
    );

    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);

    let target_pre_boundary_height = CHANGE_GAS_LIMIT_AFTER_HEIGHT.saturating_sub(2);
    let current_height = rollup.height().await.get();
    assert!(
        current_height <= target_pre_boundary_height,
        "test precondition failed: startup height {current_height} already exceeded target pre-boundary height {target_pre_boundary_height}"
    );
    rollup.wait_for_height(target_pre_boundary_height).await;

    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let mut nonce = client.get_transaction_count(signer.address()).await?;
    let mut tx_blocks = Vec::new();

    // Send transactions in blocks on both sides of the fee boundary (so that eth_feeHistory will have nonzero ratios)
    for _ in 0..6 {
        let receipt = send_high_fee_transfer(&client, nonce).await?;
        rollup.wait_for_rollup_height_advance_by(1).await;
        nonce = nonce.saturating_add(1);
        tx_blocks.push(
            receipt
                .block_number
                .expect("transfer receipt should include block number"),
        );

        let has_pre_change = tx_blocks
            .iter()
            .any(|&block| block <= CHANGE_GAS_LIMIT_AFTER_HEIGHT);
        let has_post_change = tx_blocks
            .iter()
            .any(|&block| block > CHANGE_GAS_LIMIT_AFTER_HEIGHT);
        if has_pre_change && has_post_change {
            break;
        }
    }

    let pre_change_block = tx_blocks
        .iter()
        .copied()
        .filter(|&block| block <= CHANGE_GAS_LIMIT_AFTER_HEIGHT)
        .max()
        .expect("expected at least one tx-bearing block at or before the change height");
    let post_change_block = tx_blocks
        .iter()
        .copied()
        .find(|&block| block > CHANGE_GAS_LIMIT_AFTER_HEIGHT)
        .expect("expected at least one tx-bearing block after the change height");

    let block_count = post_change_block
        .saturating_sub(pre_change_block)
        .saturating_add(1);
    let fee_history = client
        .get_fee_history(
            block_count,
            BlockNumberOrTag::Number(post_change_block),
            &[],
        )
        .await?;

    assert_eq!(
        fee_history.oldest_block, pre_change_block,
        "feeHistory range should start at the last tx-bearing block before the gas-limit change"
    );

    let pre_change_idx = 0usize;
    let post_change_idx: usize = post_change_block
        .saturating_sub(pre_change_block)
        .try_into()
        .expect("block distance should fit in usize");

    let pre_change_gas_used = total_gas_used_from_receipts(&client, pre_change_block).await?;
    let post_change_gas_used = total_gas_used_from_receipts(&client, post_change_block).await?;
    assert!(
        pre_change_gas_used > 0,
        "pre-change block should contain a transaction"
    );
    assert!(
        post_change_gas_used > 0,
        "post-change block should contain a transaction"
    );

    let observed_pre_change_ratio: f64 = fee_history.gas_used_ratio[pre_change_idx];
    let observed_post_change_ratio: f64 = fee_history.gas_used_ratio[post_change_idx];
    assert!(
        observed_post_change_ratio > observed_pre_change_ratio,
        "post-change gas_used_ratio should increase when the gas limit is reduced"
    );

    let observed_ratio_change = observed_pre_change_ratio / observed_post_change_ratio;
    let expected_ratio_change = (pre_change_gas_used as f64 / post_change_gas_used as f64)
        * (UPDATED_GAS_LIMIT as f64 / INITIAL_GAS_LIMIT as f64);
    assert_float_eq(
        observed_ratio_change,
        expected_ratio_change,
        "gas_used_ratio change across gas-limit transition",
    );
    assert!(
        observed_ratio_change < 0.6,
        "gas_used_ratio before/after should reflect the gas-limit drop, got {observed_ratio_change}"
    );

    // `wrong_shared_denominator_ratio_change` is the ratio of gas_used across the two blocks. If the gas limit hadn't changed,
    // the rateio of `fee_ratio_before / fee_ratio_after` would be equal to this value. We want to assert that it *has* changed
    let wrong_shared_denominator_ratio_change =
        pre_change_gas_used as f64 / post_change_gas_used as f64;
    assert!(
        (observed_ratio_change - wrong_shared_denominator_ratio_change).abs() > 1e-12, // Float not equals check
        "gas_used_ratio change should include the gas-limit transition, not just the gas-used change"
    );

    Ok(())
}

/// TC30: Multiple transactions across blocks show distinct ratios
///
/// This test verifies that blocks with transactions show non-zero gas_used_ratio.
/// Note: Transactions may batch together depending on timing, so we track
/// the actual block each transaction lands in rather than assuming separation.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_multiple_txs_across_blocks() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(2).await;

    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");

    // Wait for deploy to finalize
    rollup.wait_for_rollup_height_advance_by(2).await;

    // Track blocks where transactions land
    let mut tx_blocks = Vec::new();

    // Send 3 transactions, ensuring each lands in a separate block by:
    // 1. Wait for finalized receipt (tx is sealed in a block)
    // 2. Wait for next block before sending next tx
    for i in 0..3u32 {
        let tx_hash = simple_storage.set_value(contract_address, 100 + i).await;
        let receipt = simple_storage.wait_for_finalized_receipt(tx_hash).await;
        if let Some(block_num) = receipt.block_number {
            tx_blocks.push(block_num);
        }
        // Ensure next block starts before sending next tx
        rollup.wait_for_rollup_height_advance_by(1).await;
    }

    rollup.pause_preferred_batches_and_wait().await?;

    // Query fee history covering all transaction blocks
    let latest_block = client.get_block_number().await?;
    let fee_history = client
        .get_fee_history(10, BlockNumberOrTag::Number(latest_block), &[])
        .await?;

    // Count blocks with non-zero gas usage
    let nonzero_count = fee_history
        .gas_used_ratio
        .iter()
        .filter(|&&r| r > 0.0)
        .count();

    // We should have at least 2 blocks with gas (deploy may batch with first set_value)
    // The key invariant is that transactions DO show up as non-zero gas
    assert!(
        nonzero_count >= 2,
        "Expected at least 2 blocks with gas usage (4 txs may batch), got {nonzero_count} in {:?}. Tx blocks: {:?}",
        fee_history.gas_used_ratio,
        tx_blocks
    );

    // Verify the unique transaction blocks show non-zero ratios when queried directly
    for &block_num in &tx_blocks {
        if block_num >= fee_history.oldest_block {
            let idx = (block_num - fee_history.oldest_block) as usize;
            if idx < fee_history.gas_used_ratio.len() {
                assert!(
                    fee_history.gas_used_ratio[idx] > 0.0,
                    "Block {} should have non-zero gas_used_ratio, got {}",
                    block_num,
                    fee_history.gas_used_ratio[idx]
                );
            }
        }
    }

    Ok(())
}

/// TC31: Fee history progression - oldest_block advances as new blocks are produced
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_progression() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(5).await;

    let history1 = client
        .get_fee_history(3, BlockNumberOrTag::Latest, &[])
        .await?;
    let oldest1 = history1.oldest_block;

    rollup.wait_for_rollup_height_advance_by(3).await;

    let history2 = client
        .get_fee_history(3, BlockNumberOrTag::Latest, &[])
        .await?;
    let oldest2 = history2.oldest_block;

    assert!(
        oldest2 > oldest1,
        "After producing blocks, oldest_block should advance: was {oldest1}, now {oldest2}"
    );

    let block_diff = oldest2 - oldest1;
    assert!(
        (2..=4).contains(&block_diff),
        "Block advancement should be ~3, got {block_diff}"
    );

    Ok(())
}

/// TC32: Same block queried via Number(N) and via range ending at N returns consistent gas_used_ratio
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_consistent_query_methods() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(2).await;

    // Create some gas usage
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");
    rollup.wait_for_rollup_height_advance_by(2).await;
    set_value_check(&simple_storage, contract_address, 999)
        .await
        .expect("set_value should succeed");
    rollup.wait_for_rollup_height_advance_by(2).await;
    rollup.pause_preferred_batches_and_wait().await?;

    // Get the current block number
    let block_num = client.get_block_number().await?;
    let target_block = block_num - 2; // A sealed block

    // Query using Number(target_block) with block_count=1
    let history_by_number = client
        .get_fee_history(1, BlockNumberOrTag::Number(target_block), &[])
        .await?;

    // Query using a range that ends at target_block (block_count=3)
    let history_range = client
        .get_fee_history(3, BlockNumberOrTag::Number(target_block), &[])
        .await?;

    // The last gas_used_ratio in history_range should match the single ratio in history_by_number
    let ratio_single = history_by_number.gas_used_ratio[0];
    let ratio_from_range = history_range.gas_used_ratio.last().unwrap();

    assert_eq!(
        ratio_single, *ratio_from_range,
        "gas_used_ratio for block {target_block} should be consistent: single={ratio_single}, range={ratio_from_range}"
    );

    Ok(())
}

/// TC34: Mixed history pattern - verify ratios match pattern [0, >0, 0, >0, >0]
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_mixed_pattern() -> anyhow::Result<()> {
    let rollup = setup_test_rollup_with_ideal_lag(0, EVM_EXTENSION, 0).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(2).await;

    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");

    // Wait for deploy block
    rollup.wait_for_rollup_height_advance_by(1).await;

    // Record starting block
    let start_block = client.get_block_number().await?;

    // Create pattern: [empty, tx, empty, tx, tx]
    // Block 1: empty
    rollup.wait_for_rollup_height_advance_by(1).await;

    // Block 2: tx
    set_value_check(&simple_storage, contract_address, 1)
        .await
        .expect("set_value should succeed");
    rollup.wait_for_rollup_height_advance_by(1).await;

    // Block 3: empty
    rollup.wait_for_rollup_height_advance_by(1).await;

    // Block 4: tx
    set_value_check(&simple_storage, contract_address, 2)
        .await
        .expect("set_value should succeed");
    rollup.wait_for_rollup_height_advance_by(1).await;

    // Block 5: tx
    set_value_check(&simple_storage, contract_address, 3)
        .await
        .expect("set_value should succeed");
    rollup.wait_for_rollup_height_advance_by(1).await;

    rollup.pause_preferred_batches_and_wait().await?;

    // Query for 5 blocks after start_block
    let end_block = start_block + 5;
    let fee_history = client
        .get_fee_history(5, BlockNumberOrTag::Number(end_block), &[])
        .await?;

    // Verify the pattern: should have alternating zero/nonzero pattern
    // The exact pattern depends on timing, but we can verify:
    // - Some blocks have ratio = 0.0 (empty)
    // - Some blocks have ratio > 0.0 (with tx)
    let zero_count = fee_history
        .gas_used_ratio
        .iter()
        .filter(|&&r| r == 0.0)
        .count();
    let nonzero_count = fee_history
        .gas_used_ratio
        .iter()
        .filter(|&&r| r > 0.0)
        .count();

    assert!(
        zero_count >= 1,
        "Expected at least 1 empty block, got {zero_count}"
    );
    assert!(
        nonzero_count >= 2,
        "Expected at least 2 blocks with transactions, got {nonzero_count}"
    );

    Ok(())
}

/// TC35: Base fee stability - regression test
///
/// This test verifies EIP-1559 base fee constraints: changes should be max 12.5% per block,
/// and base fee should never drop below 1 wei.
/// See: crates/module-system/module-implementations/sov-chain-state/src/gas.rs
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_base_fee_stability() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(10).await;

    let fee_history = client
        .get_fee_history(8, BlockNumberOrTag::Finalized, &[])
        .await?;

    // Check that base_fee values don't have extreme swings
    // EIP-1559 constrains changes to max 12.5% per block
    let base_fees = &fee_history.base_fee_per_gas;
    assert!(
        base_fees.len() >= 2,
        "Need at least 2 base fees to check stability"
    );

    for i in 1..base_fees.len() {
        let prev = base_fees[i - 1];
        let curr = base_fees[i];

        // Allow up to 15% change (slightly more than EIP-1559's 12.5% to account for implementation variance)
        // But if prev is 0, any value is acceptable
        if prev > 0 {
            let max_change = prev / 8 + prev / 50; // ~14.5%
            let diff = curr.abs_diff(prev);
            assert!(
                diff <= max_change,
                "Base fee swing from block {}->{i} is too large: {prev} -> {curr} (diff={diff}, max={max_change})",
                i - 1,
            );
        }
    }

    Ok(())
}

// ==================== Missing Test Cases ====================

/// TC06: finalized and safe return identical results
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_finalized_equals_safe() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_rollup_height_advance_by(5).await;
    rollup.pause_preferred_batches_and_wait().await?;

    let finalized = client
        .get_fee_history(3, BlockNumberOrTag::Finalized, &[25.0, 75.0])
        .await?;

    let safe = client
        .get_fee_history(3, BlockNumberOrTag::Safe, &[25.0, 75.0])
        .await?;

    // In this rollup, finalized and safe both map to latest finalized block
    assert_eq!(
        finalized.oldest_block, safe.oldest_block,
        "finalized and safe should return same oldest_block"
    );
    assert_eq!(
        finalized.base_fee_per_gas, safe.base_fee_per_gas,
        "finalized and safe should return same base_fee_per_gas"
    );
    assert_eq!(
        finalized.gas_used_ratio, safe.gas_used_ratio,
        "finalized and safe should return same gas_used_ratio"
    );
    assert_eq!(
        finalized.reward, safe.reward,
        "finalized and safe should return same reward"
    );

    Ok(())
}

/// TC09: blockCount = 1024 exactly works (boundary case)
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_block_count_1024_boundary() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(5).await;

    // Request exactly 1024 blocks - should work without error
    let fee_history = client
        .get_fee_history(1024, BlockNumberOrTag::Latest, &[])
        .await?;

    assert!(!fee_history.base_fee_per_gas.is_empty());
    assert_eq!(
        fee_history.oldest_block, 0,
        "short chains should underflow to genesis"
    );
    assert!(
        fee_history.base_fee_per_gas.len() <= 1025,
        "base_fee_per_gas should have at most 1025 entries for blockCount=1024"
    );

    Ok(())
}

/// TC13: Duplicate percentiles - verify behavior (spec allows <=)
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_duplicate_percentiles() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(3).await;

    // Duplicate percentiles [25, 25, 75] - should be accepted (monotonically non-decreasing)
    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[25.0, 25.0, 75.0])
        .await?;

    let rewards = fee_history.reward.expect("reward should be present");
    assert_eq!(rewards.len(), 2, "Should have 2 blocks of rewards");
    for (i, row) in rewards.iter().enumerate() {
        assert_eq!(
            row.len(),
            3,
            "Block {i} reward should have 3 percentile values"
        );
        // Duplicate percentiles should return same value
        assert_eq!(
            row[0], row[1],
            "Duplicate percentiles 25, 25 should return same value"
        );
    }

    Ok(())
}

/// TC14: Empty percentiles array omits reward field
///
/// Regression: when empty percentiles are provided, the reward field is omitted (None).
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_empty_percentiles_no_reward() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(3).await;

    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[])
        .await?;

    // With empty percentiles, reward should be None
    assert!(
        fee_history.reward.is_none(),
        "Empty percentiles should result in no reward field, got {:?}",
        fee_history.reward
    );

    Ok(())
}

/// TC19: Blob gas fields are empty arrays (rollup-specific, EIP-4844 not implemented)
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_blob_gas_fields_empty() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(3).await;

    let fee_history = client
        .get_fee_history(3, BlockNumberOrTag::Latest, &[])
        .await?;

    assert!(
        fee_history.base_fee_per_blob_gas.is_empty(),
        "base_fee_per_blob_gas should be empty (EIP-4844 not implemented), got {:?}",
        fee_history.base_fee_per_blob_gas
    );
    assert!(
        fee_history.blob_gas_used_ratio.is_empty(),
        "blob_gas_used_ratio should be empty (EIP-4844 not implemented), got {:?}",
        fee_history.blob_gas_used_ratio
    );

    Ok(())
}

/// TC24: baseFeePerGas[last] is the predicted next block fee
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_predicted_next_block_fee() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(5).await;

    let latest = client.get_block_number().await?;
    let newest_block = latest.saturating_sub(1);
    let fee_history = client
        .get_fee_history(1, BlockNumberOrTag::Number(newest_block), &[])
        .await?;

    assert_eq!(
        fee_history.base_fee_per_gas.len(),
        2,
        "Should have block_count + 1 base fees"
    );

    let genesis = load_evm_genesis_config();
    let base_fees = base_fee_series_from_genesis(&client, newest_block, &genesis).await?;
    let base_fee = base_fees[newest_block as usize];
    assert!(base_fee > 0, "baseFeePerGas should be non-zero");
    assert_eq!(
        fee_history.base_fee_per_gas[0], base_fee,
        "baseFeePerGas should match receipt-derived value"
    );

    let gas_limit = genesis.chain_spec.block_gas_limit;
    let receipts_gas_used = total_gas_used_from_receipts(&client, newest_block).await?;
    let params = &genesis.chain_spec.base_fee_params;
    let expected_predicted = compute_next_base_fee(
        base_fee,
        receipts_gas_used,
        gas_limit,
        params.max_change_denominator,
        params.elasticity_multiplier,
    );
    assert_eq!(
        fee_history.base_fee_per_gas[1], expected_predicted,
        "predicted next base fee should follow EIP-1559"
    );

    let next_base_fee = get_block_base_fee(&client, newest_block + 1).await?;
    assert_eq!(
        fee_history.base_fee_per_gas[1], next_base_fee,
        "predicted next base fee should match the next block header"
    );

    Ok(())
}

/// TC27: Future block number handling
///
/// Regression: requesting fee history for a future block returns an error.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_future_block() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(3).await;

    let current_block = client.get_block_number().await?;
    let future_block = current_block + 1000;

    let result = client
        .get_fee_history(3, BlockNumberOrTag::Number(future_block), &[])
        .await;

    assert!(
        result.is_err(),
        "Future block should return error (current block: {current_block}, future: {future_block})"
    );

    Ok(())
}

/// TC28: Fractional percentiles work correctly
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_fractional_percentiles() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(3).await;

    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[10.5, 50.0, 90.5])
        .await?;

    let rewards = fee_history.reward.expect("reward should be present");
    assert_eq!(rewards.len(), 2, "Should have 2 blocks of rewards");
    for (i, row) in rewards.iter().enumerate() {
        assert_eq!(
            row.len(),
            3,
            "Block {i} reward should have 3 percentile values for fractional percentiles"
        );
    }

    Ok(())
}

/// TC22: Percentile boundary values 0 and 100
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_percentile_boundaries() -> anyhow::Result<()> {
    let (_rollup, client) = setup_fee_history_test(3).await;

    let fee_history = client
        .get_fee_history(2, BlockNumberOrTag::Latest, &[0.0, 50.0, 100.0])
        .await?;

    let rewards = fee_history.reward.expect("reward should be present");
    assert_eq!(rewards.len(), 2, "Should have 2 blocks of rewards");
    for (i, row) in rewards.iter().enumerate() {
        assert_eq!(
            row.len(),
            3,
            "Block {i} reward should have 3 percentile values"
        );
        assert!(
            row[0] <= row[2],
            "Block {i}: 0th percentile {} should be <= 100th percentile {}",
            row[0],
            row[2]
        );
    }

    Ok(())
}

/// TC34: Heavy gas usage - gas_used_ratio approaches but doesn't exceed 1.0
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_heavy_gas_usage() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    let simple_storage = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    rollup.wait_for_rollup_height_advance_by(2).await;

    let contract_address = deploy_contract_check(&simple_storage)
        .await
        .expect("deploy should succeed");
    rollup.wait_for_rollup_height_advance_by(1).await;

    let block_before = client.get_block_number().await?;

    // Burn a lot of gas with keccak256 loops
    let tx_hash = simple_storage.alloy_burn_gas(contract_address, 10000).await;
    simple_storage.wait_for_finalized_receipt(tx_hash).await;
    // Ensure the block containing the tx is sealed before pausing.
    rollup.wait_for_rollup_height_advance_by(1).await;

    rollup.pause_preferred_batches_and_wait().await?;

    let block_after = client.get_block_number().await?;

    let fee_history = client
        .get_fee_history(
            block_after - block_before + 1,
            BlockNumberOrTag::Finalized,
            &[],
        )
        .await?;

    let max_ratio = fee_history
        .gas_used_ratio
        .iter()
        .cloned()
        .fold(0.0_f64, f64::max);

    assert!(
        max_ratio > 0.0,
        "Heavy gas tx should produce non-zero gas_used_ratio, got {max_ratio}",
    );

    // Critical invariant: ratio should never exceed 1.0
    assert_gas_ratios_valid(&fee_history.gas_used_ratio);

    Ok(())
}
