use alloy_consensus::{transaction::Recovered, Signed, TxEip4844Variant, TxEnvelope};
use alloy_primitives::TxKind;
use alloy_primitives::{BlockNumber, Sealed};
use alloy_primitives::{B256, U256};
use alloy_rpc_types::{Header, TransactionRequest};
use reth_rpc_eth_types::EthResult;
use revm::context::{BlockEnv, TransactionType, TxEnv};

use alloy_consensus::TxEip4844;

use crate::evm::primitive_types::TransactionSigned;
pub type PrimitiveTransaction = alloy_consensus::EthereumTypedTransaction<TxEip4844>;

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
        ..Default::default()
    };

    Ok(env)
}

pub(crate) fn from_primitive_with_hash(
    primitive_header: Sealed<alloy_consensus::Header>,
) -> Header {
    Header::from_consensus(primitive_header, None, None)
}

/// copy from [`reth_rpc_types_compat::transaction::from_recovered_with_block_context`]
pub(crate) fn from_recovered_with_block_context(
    tx: Recovered<TransactionSigned>,
    block_hash: Option<B256>,
    block_number: BlockNumber,
    tx_index: U256,
) -> alloy_rpc_types::Transaction {
    let block_number = Some(block_number);
    let transaction_index = Some(tx_index.to::<u64>());

    let signer = tx.signer();
    let signed_tx = tx.into_inner();

    let (tx, sig, hash) = signed_tx.into_signed().into_parts();
    let tx = match tx {
        PrimitiveTransaction::Legacy(tx) => {
            TxEnvelope::Legacy(Signed::new_unchecked(tx, sig, hash))
        }
        PrimitiveTransaction::Eip2930(tx) => {
            TxEnvelope::Eip2930(Signed::new_unchecked(tx, sig, hash))
        }
        PrimitiveTransaction::Eip1559(tx) => {
            TxEnvelope::Eip1559(Signed::new_unchecked(tx, sig, hash))
        }
        PrimitiveTransaction::Eip4844(tx) => TxEnvelope::Eip4844(Signed::new_unchecked(
            TxEip4844Variant::TxEip4844(tx),
            sig,
            hash,
        )),
        PrimitiveTransaction::Eip7702(tx) => {
            TxEnvelope::Eip7702(Signed::new_unchecked(tx, sig, hash))
        }
    };

    alloy_rpc_types::Transaction {
        inner: Recovered::new_unchecked(tx, signer),
        block_hash,
        block_number,
        transaction_index,
        effective_gas_price: None,
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
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
}
