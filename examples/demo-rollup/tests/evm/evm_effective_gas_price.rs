use alloy_primitives::U256;
use alloy_rpc_types_eth::{Block, BlockTransactions};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;

use crate::evm::evm_test_helper::{
    create_simple_storage_client, setup_test_rollup, EVM_EXTENSION, HIGH_PRIORITY_FEE_PER_GAS,
    MAX_FEE_PER_GAS, SENDER_PRIV_KEY,
};

#[tokio::test(flavor = "multi_thread")]
async fn eip1559_tx_and_receipt_have_valid_effective_gas_price() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(2).await;

    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let sender = client.address();
    let balance_before = client.eth_get_balance(sender).await;

    let mut tx = client.make_tx(Some(sender), None);
    tx = tx
        .value(U256::from(1u64))
        .max_fee_per_gas(MAX_FEE_PER_GAS)
        .max_priority_fee_per_gas(HIGH_PRIORITY_FEE_PER_GAS);
    tx.gas = None;
    let estimated_gas_limit = client.eth_estimate_gas(tx.clone()).await;
    let advertised_ceiling = U256::from(estimated_gas_limit)
        .checked_mul(U256::from(MAX_FEE_PER_GAS))
        .and_then(|gas_cost| gas_cost.checked_add(U256::from(1u64)))
        .ok_or_else(|| anyhow::anyhow!("self-transfer affordability ceiling overflow"))?;
    assert!(
        advertised_ceiling < balance_before,
        "test precondition failed: self-transfer ceiling {advertised_ceiling} must be below sender balance {balance_before}"
    );
    tx = tx.gas_limit(estimated_gas_limit);

    let receipt = client
        .send_tx_and_wait_finalized(tx)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    let tx_hash = receipt.transaction_hash;
    let balance_after = client.eth_get_balance(sender).await;
    let tx_response = client
        .transaction(tx_hash)
        .await
        .ok_or_else(|| anyhow::anyhow!("missing transaction response for {tx_hash:?}"))?;

    assert!(
        tx_response.effective_gas_price.is_some(),
        "eth_getTransactionByHash should include effective_gas_price"
    );
    assert!(
        tx_response.effective_gas_price.unwrap() > 0,
        "effective_gas_price should be positive"
    );
    assert!(
        receipt.effective_gas_price > 0,
        "receipt effective_gas_price should be positive"
    );
    assert_eq!(
        tx_response.effective_gas_price,
        Some(receipt.effective_gas_price),
        "eth_getTransactionByHash effective_gas_price should match receipt"
    );

    let block_number = receipt
        .block_number
        .ok_or_else(|| anyhow::anyhow!("finalized receipt should include block_number"))?;
    let block_id = format!("0x{block_number:x}");
    let full_block: Block = client
        .ws
        .request("eth_getBlockByNumber", rpc_params![Some(block_id), true])
        .await?;

    let block_tx = match full_block.transactions {
        BlockTransactions::Full(txs) => txs
            .into_iter()
            .find(|tx| *tx.inner.hash() == tx_hash)
            .ok_or_else(|| anyhow::anyhow!("tx {tx_hash:?} not found in full block response"))?,
        BlockTransactions::Hashes(_) | BlockTransactions::Uncle => {
            anyhow::bail!("expected full tx objects from eth_getBlockByNumber(..., true)")
        }
    };

    assert!(
        block_tx.effective_gas_price.is_some(),
        "eth_getBlockByNumber(full) tx should include effective_gas_price"
    );
    assert!(
        block_tx.effective_gas_price.unwrap() > 0,
        "block tx effective_gas_price should be positive"
    );
    assert_eq!(
        block_tx.effective_gas_price,
        Some(receipt.effective_gas_price),
        "eth_getBlockByNumber(full) tx effective_gas_price should match receipt"
    );

    let gas_cost = U256::from(receipt.gas_used) * U256::from(receipt.effective_gas_price);
    let actual_spent = balance_before
        .checked_sub(balance_after)
        .ok_or_else(|| anyhow::anyhow!("sender balance should decrease"))?;
    assert_eq!(
        actual_spent, gas_cost,
        "sender balance delta should exactly match receipt-implied gas cost for self-transfer"
    );

    Ok(())
}
