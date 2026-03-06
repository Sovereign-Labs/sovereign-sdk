use alloy::consensus::Transaction as TransactionTrait;
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types_eth::BlockNumberOrTag;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde_json::Value;
use sov_evm_test_utils::SimpleStorage;

use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::create_simple_storage_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;

/// Test that demonstrates the BASEFEE opcode returns the actual block base fee.
#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_opcode_returns_nonzero() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client(rollup.http_addr);

    // Deploy the SimpleStorage contract
    let contract = SimpleStorage::deploy(client.clone()).await?;

    // Call getBaseFee() via eth_call - this uses the BASEFEE opcode internally
    let base_fee_from_opcode: U256 = contract.getBaseFee().call().await?;

    // Get the actual block's base_fee_per_gas from the block header
    let block = client
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .expect("Block should exist");
    let expected_base_fee = block
        .header
        .base_fee_per_gas
        .expect("Block should have base fee");

    // Verify the block actually has a non-zero base fee (from genesis config)
    assert!(
        expected_base_fee > 0,
        "Block base_fee_per_gas should be > 0 (genesis sets initial_base_fee: 10)"
    );

    // BASEFEE should match the block header value.
    assert_eq!(
        base_fee_from_opcode,
        U256::from(expected_base_fee),
        "BASEFEE opcode should return the actual block base fee. \
         Got {base_fee_from_opcode} from opcode but block header has {expected_base_fee}",
    );

    Ok(())
}

/// Test that verifies the transaction gasPrice field matches effectiveGasPrice for EIP-1559 txs.
#[tokio::test(flavor = "multi_thread")]
async fn test_transaction_gas_price_uses_effective_price() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client(rollup.http_addr);
    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    // Deploy a contract (this sends an EIP-1559 transaction)
    let contract = SimpleStorage::deploy(client.clone()).await?;

    let sender = ws_client.address();
    let balance_before = ws_client.eth_get_balance(sender).await;

    // Send another transaction so we have a confirmed tx to check
    let tx = contract.set(U256::from(12345)).send().await?;
    let receipt = tx.get_receipt().await?;
    let tx_hash = receipt.transaction_hash;
    let balance_after = ws_client.eth_get_balance(sender).await;

    // Make a raw JSON-RPC call to get the transaction and check the gasPrice field
    let tx_json: Value = ws_client
        .ws
        .request("eth_getTransactionByHash", rpc_params![tx_hash])
        .await?;

    // Extract gasPrice from the JSON response
    let gas_price_str = tx_json["gasPrice"]
        .as_str()
        .expect("Transaction should have gasPrice field");
    let gas_price = U256::from_str_radix(gas_price_str.trim_start_matches("0x"), 16)
        .expect("gasPrice should be valid hex");

    // Get the effective gas price from the receipt (this is correctly calculated)
    let effective_gas_price = receipt.effective_gas_price;

    // Get the transaction to access max_fee_per_gas
    let rpc_tx = client
        .get_transaction_by_hash(tx_hash)
        .await?
        .expect("Transaction should exist");
    let max_fee_per_gas = rpc_tx.max_fee_per_gas();

    // For EIP-1559 transactions, gasPrice should equal effectiveGasPrice.
    assert_eq!(
        gas_price, effective_gas_price,
        "Transaction gasPrice ({gas_price}) should match receipt effectiveGasPrice ({effective_gas_price}). \
         maxFeePerGas was {max_fee_per_gas}.",
    );

    let gas_cost = U256::from(receipt.gas_used) * U256::from(receipt.effective_gas_price);
    let actual_spent = balance_before
        .checked_sub(balance_after)
        .ok_or_else(|| anyhow::anyhow!("sender balance should decrease"))?;
    assert_eq!(
        actual_spent, gas_cost,
        "sender balance delta should exactly match receipt-implied gas cost for zero-value tx"
    );

    Ok(())
}
