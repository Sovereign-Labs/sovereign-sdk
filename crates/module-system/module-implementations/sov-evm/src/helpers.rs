use alloy_consensus::transaction::Recovered;
use alloy_primitives::BlockNumber;
use alloy_primitives::TxKind;
use alloy_primitives::B256;
use alloy_rpc_types::{TransactionInfo, TransactionRequest};
use revm::context::{BlockEnv, TransactionType, TxEnv};
use sov_rpc_eth_types::EthResult;

use crate::evm::primitive_types::TransactionSigned;

// https://github.com/paradigmxyz/reth/blob/d8677b4146f77c7c82d659c59b79b38caca78778/crates/rpc/rpc/src/eth/revm_utils.rs#L201
// it is `pub(crate)` only for tests
pub(crate) fn prepare_call_env(
    block_env: &BlockEnv,
    request: TransactionRequest,
) -> EthResult<TxEnv> {
    let TransactionRequest {
        from,
        to,
        gas,
        value,
        input,
        nonce,
        access_list,
        chain_id,
        ..
    } = request;

    let gas_limit = gas.unwrap_or(block_env.gas_limit);

    let env = TxEnv {
        tx_type: TransactionType::Eip1559.into(),
        gas_limit,
        nonce: nonce.unwrap_or_default(),
        caller: from.unwrap_or_default(),
        gas_price: 0,
        gas_priority_fee: None,
        kind: to.unwrap_or(TxKind::Create),
        value: value.unwrap_or_default(),
        data: input.try_into_unique_input()?.unwrap_or_default(),
        chain_id,
        access_list: access_list.unwrap_or_default(),
        // Default values
        blob_hashes: vec![],
        max_fee_per_blob_gas: 0,
        authorization_list: vec![],
    };

    Ok(env)
}

/// copy from [`reth_rpc_types_compat::transaction::from_recovered_with_block_context`]
pub(crate) fn from_recovered_with_block_context(
    tx: Recovered<TransactionSigned>,
    block_hash: Option<B256>,
    block_number: BlockNumber,
    tx_index: u64,
    base_fee: Option<u64>,
) -> alloy_rpc_types::Transaction {
    let tx_info = TransactionInfo {
        base_fee,
        block_hash,
        block_number: Some(block_number),
        index: Some(tx_index),
        // Default value, because hash is in the tx.
        hash: None,
    };
    alloy_rpc_types::Transaction::from_transaction(tx.convert(), tx_info)
}

#[cfg(test)]
mod tests {
    use alloy_consensus::{EthereumTxEnvelope, Signed, TxEip1559};
    use alloy_primitives::Signature;
    use alloy_primitives::{Address, B256, U256};
    use revm::context::TransactTo;

    use super::*;

    // TODO: Needs more complex tests later
    #[test]
    fn prepare_call_env_conversion() {
        let from = Address::random();
        let to = Address::random();
        let request = TransactionRequest {
            from: Some(from),
            to: Some(TxKind::Call(to)),
            gas_price: Some(100),
            gas: Some(200),
            value: Some(U256::from(300u64)),
            nonce: Some(1),
            chain_id: Some(1),
            transaction_type: Some(2),
            ..Default::default()
        };

        let block_env = BlockEnv::default();

        let tx_env = prepare_call_env(&block_env, request).unwrap();
        let expected = TxEnv {
            tx_type: TransactionType::Eip1559.into(),
            caller: from,
            gas_price: 0,
            gas_limit: 200,
            kind: TransactTo::Call(to),
            value: U256::from(300u64),
            chain_id: Some(1),
            nonce: 1,
            ..Default::default()
        };

        assert_eq!(tx_env.caller, expected.caller);
        assert_eq!(tx_env.gas_limit, expected.gas_limit);
        assert_eq!(tx_env.gas_price, expected.gas_price);
        assert_eq!(tx_env.gas_priority_fee, expected.gas_priority_fee);
        assert_eq!(tx_env.kind.is_create(), expected.kind.is_create());
        assert_eq!(tx_env.value, expected.value);
        assert_eq!(tx_env.data, expected.data);
        assert_eq!(tx_env.chain_id, expected.chain_id);
        assert_eq!(tx_env.nonce, expected.nonce);
        assert_eq!(tx_env.access_list, expected.access_list);
    }

    #[test]
    fn from_recovered_with_block_context_uses_base_fee_for_effective_gas_price() {
        let tx = TxEip1559 {
            max_fee_per_gas: 100,
            max_priority_fee_per_gas: 2,
            ..Default::default()
        };
        let recovered = Recovered::new_unchecked(
            EthereumTxEnvelope::Eip1559(Signed::new_unchecked(
                tx,
                Signature::test_signature(),
                Default::default(),
            )),
            Address::ZERO,
        );

        let with_base_fee =
            from_recovered_with_block_context(recovered.clone(), Some(B256::ZERO), 1, 0, Some(10));
        // min(max_priority_fee_per_gas, max_fee_per_gas - base_fee) + base_fee
        assert_eq!(with_base_fee.effective_gas_price, Some(12));

        let without_base_fee =
            from_recovered_with_block_context(recovered, Some(B256::ZERO), 1, 0, None);
        // Fallback behavior when base fee is unavailable.
        assert_eq!(without_base_fee.effective_gas_price, Some(100));
    }
}
