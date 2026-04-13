use alloy_consensus::{transaction::Recovered, Transaction as TransactionTrait, TxReceipt};
use alloy_primitives::{Address, TxKind};
use alloy_rpc_types::{Log, ReceiptEnvelope, ReceiptWithBloom, TransactionReceipt};
use sov_modules_api::da::Time;
use sov_modules_api::Amount;
use sov_rpc_eth_types::LogWithExecutionTimestamp;

use crate::evm::primitive_types::{Receipt, TransactionSigned, TxSignedAndRecovered};
use crate::primitive_types::MaybeSealedBlock;
use crate::sov_fee_and_gas_utils::is_actual_fee_projection_height_active;

pub(crate) fn maybe_actual_effective_gas_price(
    block_number: u64,
    gas_used: u64,
    fee_paid: Option<Amount>,
    base_fee_per_gas: Option<u64>,
) -> Option<u128> {
    if !is_actual_fee_projection_height_active(block_number) || gas_used == 0 {
        return None;
    }

    // Keep zero-fee projection: when metadata says no fee was charged, RPC must return 0 here.
    // For non-zero fee, we report the block header base fee (primary gas-price dimension).
    // Invariant: this header value must match the gas meter's `gas_price[0]`; divergence is a bug.
    match fee_paid {
        Some(Amount::ZERO) => Some(0),
        Some(_) => base_fee_per_gas.map(u128::from),
        None => None,
    }
}

// modified from: https://github.com/paradigmxyz/reth many times
pub(crate) fn build_rpc_receipt(
    block: &MaybeSealedBlock,
    tx: TxSignedAndRecovered,
    tx_number: u64,
    receipt: Receipt,
    time: Time,
    fee_paid: Option<Amount>,
) -> TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>> {
    let transaction: Recovered<TransactionSigned> = tx.into();
    let from = transaction.signer();

    let block_hash = block.hash();
    let block_number = Some(block.number());
    let transaction_index = tx_number
        .checked_sub(block.tx_range().start)
        .expect("tx_number is within block.tx_range()");

    let transaction_hash = receipt.transaction_hash;
    let logs_bloom = receipt.receipt.bloom();

    let time = time.as_millis().try_into().unwrap_or_default();
    let logs: Vec<LogWithExecutionTimestamp> = receipt
        .receipt
        .logs
        .into_iter()
        .enumerate()
        .map(|(tx_log_idx, log)| LogWithExecutionTimestamp {
            log: Log {
                inner: log,
                block_hash,
                block_number,
                block_timestamp: Some(block.timestamp()),
                transaction_hash: Some(transaction_hash),
                transaction_index: Some(transaction_index),
                log_index: Some(receipt.log_index_start + tx_log_idx as u64),
                removed: false,
            },
            time_executed_ms: time,
        })
        .collect();

    let rpc_receipt = alloy_rpc_types::Receipt {
        status: receipt.receipt.success.into(),
        cumulative_gas_used: receipt.receipt.cumulative_gas_used,
        logs,
    };

    let (contract_address, to) = match transaction.kind() {
        TxKind::Create => (Some(from.create(transaction.nonce())), None),
        TxKind::Call(addr) => (None, Some(Address(*addr))),
    };

    // EIP-1559 effective gas price calculation (https://github.com/ethereum/EIPs/blob/0d31c18725202ae8bbfb82b8d3d028ad1810d360/EIPS/eip-1559.md?plain=1#L222-L224):
    //   priority_fee_per_gas = min(transaction.max_priority_fee_per_gas,
    //                              transaction.max_fee_per_gas - block.base_fee_per_gas)
    //   effective_gas_price = priority_fee_per_gas + block.base_fee_per_gas
    let eip_1559_effective_gas_price = transaction
        .inner()
        .effective_gas_price(block.maybe_partial_header().base_fee_per_gas);

    // Once activated, keep zero-fee semantics from metered metadata and otherwise
    // report the primary gas-price dimension (EVM base fee from header).
    let effective_gas_price = maybe_actual_effective_gas_price(
        block.number(),
        receipt.gas_used,
        fee_paid,
        block.maybe_partial_header().base_fee_per_gas,
    )
    // Keep compatibility for historical data where metadata may be missing.
    .unwrap_or(eip_1559_effective_gas_price);

    TransactionReceipt {
        inner: ReceiptEnvelope::Eip1559(ReceiptWithBloom::new(rpc_receipt, logs_bloom)),
        transaction_hash,
        transaction_index: Some(transaction_index),
        block_hash,
        block_number,
        gas_used: receipt.gas_used,
        effective_gas_price,
        blob_gas_used: None,
        blob_gas_price: None,
        from,
        to,
        contract_address,
    }
}

#[cfg(test)]
mod tests {
    use sov_modules_api::macros::config_value;

    use super::*;

    #[test]
    fn maybe_actual_effective_gas_price_uses_zero_paid_fee() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
        let active_block = activation_height
            .checked_add(1)
            .expect("activation height must be strictly below u64::MAX");

        assert_eq!(
            maybe_actual_effective_gas_price(active_block, 21_000, Some(Amount::ZERO), Some(123)),
            Some(0)
        );
    }

    #[test]
    fn maybe_actual_effective_gas_price_ignores_zero_paid_fee_before_activation() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");

        assert_eq!(
            maybe_actual_effective_gas_price(
                activation_height,
                21_000,
                Some(Amount::ZERO),
                Some(123),
            ),
            None
        );
    }

    #[test]
    fn maybe_actual_effective_gas_price_uses_base_fee_for_non_zero_paid_fee() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
        let active_block = activation_height
            .checked_add(1)
            .expect("activation height must be strictly below u64::MAX");

        assert_eq!(
            maybe_actual_effective_gas_price(
                active_block,
                21_000,
                Some(Amount::new(100_000)),
                Some(42),
            ),
            Some(42)
        );
    }

    #[test]
    fn maybe_actual_effective_gas_price_falls_back_when_base_fee_missing() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
        let active_block = activation_height
            .checked_add(1)
            .expect("activation height must be strictly below u64::MAX");

        assert_eq!(
            maybe_actual_effective_gas_price(
                active_block,
                21_000,
                Some(Amount::new(100_000)),
                None,
            ),
            None
        );
    }
}
