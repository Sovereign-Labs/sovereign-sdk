use alloy_consensus::transaction::Transaction;
use alloy_consensus::{Signed, TxEnvelope};
use alloy_primitives::TxKind;
use alloy_primitives::{BlockNumber, Sealed};
use alloy_primitives::{B256, U256};
use alloy_rpc_types::{Header, TransactionRequest};
use reth_primitives::{Recovered, Transaction as PrimitiveTransaction, TransactionSigned};
use reth_rpc_convert::{CallFees, EthTxEnvError};
use reth_rpc_eth_types::EthResult;
use revm::context::{BlockEnv, TransactionType, TxEnv};

// https://github.com/paradigmxyz/reth/blob/d8677b4146f77c7c82d659c59b79b38caca78778/crates/rpc/rpc/src/eth/revm_utils.rs#L201
// it is `pub(crate)` only for tests
pub(crate) fn prepare_call_env(
    block_env: &BlockEnv,
    request: TransactionRequest,
) -> EthResult<TxEnv> {
    let TransactionRequest {
        from,
        to,
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        gas,
        value,
        input,
        nonce,
        access_list,
        chain_id,
        ..
    } = request;

    let CallFees {
        max_priority_fee_per_gas,
        gas_price,
        ..
    } = CallFees::ensure_fees(
        gas_price.map(U256::from),
        max_fee_per_gas.map(U256::from),
        max_priority_fee_per_gas.map(U256::from),
        U256::from(block_env.basefee),
        // EIP-4844 related params
        None,
        None,
        None,
    )
    .map_err(Into::<EthTxEnvError>::into)?;

    let gas_limit = gas.unwrap_or(block_env.gas_limit);

    let env = TxEnv {
        tx_type: TransactionType::Eip1559.into(),
        gas_limit,
        nonce: nonce.unwrap_or_default(),
        caller: from.unwrap_or_default(),
        gas_price: gas_price.to::<u128>(),
        gas_priority_fee: max_priority_fee_per_gas.map(|g| g.to::<u128>()),
        kind: to.unwrap_or(TxKind::Create),
        value: value.unwrap_or_default(),
        data: input.try_into_unique_input()?.unwrap_or_default(),
        chain_id,
        access_list: access_list.unwrap_or_default(),
        // EIP-4844 related fields:
        blob_hashes: Default::default(),
        max_fee_per_blob_gas: 0,
        // EIP-7702: TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1132
        authorization_list: vec![],
    };

    Ok(env)
}

pub(crate) fn from_primitive_with_hash(
    primitive_header: Sealed<alloy_consensus::Header>,
) -> Header {
    Header::from_consensus(primitive_header, None, None)
}

/// copy from [`reth_rpc_types_compat::transaction::from_recovered_with_block_context`]
pub fn from_recovered_with_block_context(
    tx: Recovered<TransactionSigned>,
    block_hash: B256,
    block_number: BlockNumber,
    base_fee: Option<u64>,
    tx_index: U256,
) -> alloy_rpc_types::Transaction {
    let block_hash = Some(block_hash);
    let block_number = Some(block_number);
    let transaction_index = Some(tx_index.to::<u64>());

    let signer = tx.signer();
    let signed_tx = tx.into_inner();

    let effective_gas_price = signed_tx.effective_gas_price(base_fee);
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
        PrimitiveTransaction::Eip4844(_) => {
            panic!("EIP-4844 transactions are not supported by the rollup");
        }
        PrimitiveTransaction::Eip7702(_) => {
            // TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1132
            panic!("EIP-7702 transactions are not yet supported by the rollup")
        }
    };

    alloy_rpc_types::Transaction {
        inner: Recovered::new_unchecked(tx, signer),
        block_hash,
        block_number,
        transaction_index,
        effective_gas_price: Some(effective_gas_price),
    }
}
