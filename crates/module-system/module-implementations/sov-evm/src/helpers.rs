use alloy_consensus::transaction::Transaction;
use alloy_consensus::{Signed, TxEnvelope};
use alloy_primitives::TxKind;
use alloy_primitives::{BlockNumber, Sealed};
use alloy_primitives::{B256, U256};
use alloy_rpc_types::{Header, TransactionRequest};
use reth_primitives::{Transaction as PrimitiveTransaction, TransactionSignedEcRecovered};
use reth_rpc_eth_types::revm_utils::CallFees;
use reth_rpc_eth_types::EthResult;
use revm::primitives::{BlockEnv, TxEnv};

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
        block_env.basefee,
        // EIP-4844 related params
        None,
        None,
        None,
    )?;

    let gas_limit = gas.unwrap_or_else(|| block_env.gas_limit.min(U256::from(u64::MAX)).to());

    let env = TxEnv {
        gas_limit,
        nonce,
        caller: from.unwrap_or_default(),
        gas_price,
        gas_priority_fee: max_priority_fee_per_gas,
        transact_to: to.unwrap_or(TxKind::Create),
        value: value.unwrap_or_default(),
        data: input.try_into_unique_input()?.unwrap_or_default(),
        chain_id,
        access_list: access_list.unwrap_or_default().into(),
        // EIP-4844 related fields:
        blob_hashes: Default::default(),
        max_fee_per_blob_gas: None,
        // EIP-7702: TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1132
        authorization_list: None,
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
    tx: TransactionSignedEcRecovered,
    block_hash: B256,
    block_number: BlockNumber,
    base_fee: Option<u64>,
    tx_index: U256,
) -> alloy_rpc_types::Transaction {
    let block_hash = Some(block_hash);
    let block_number = Some(block_number);
    let transaction_index = Some(tx_index.to::<u64>());

    let signer = tx.signer();
    let signed_tx = tx.into_signed();

    let effective_gas_price = signed_tx.effective_gas_price(base_fee);
    let tx = match signed_tx.transaction {
        PrimitiveTransaction::Legacy(tx) => TxEnvelope::Legacy(Signed::new_unchecked(
            tx,
            signed_tx.signature,
            signed_tx.hash,
        )),
        PrimitiveTransaction::Eip2930(tx) => TxEnvelope::Eip2930(Signed::new_unchecked(
            tx,
            signed_tx.signature,
            signed_tx.hash,
        )),
        PrimitiveTransaction::Eip1559(tx) => TxEnvelope::Eip1559(Signed::new_unchecked(
            tx,
            signed_tx.signature,
            signed_tx.hash,
        )),
        PrimitiveTransaction::Eip4844(_) => {
            panic!("EIP-4844 transactions are not supported by the rollup");
        }
        PrimitiveTransaction::Eip7702(_) => {
            // TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1132
            panic!("EIP-7702 transactions are not yet supported by the rollup")
        }
    };

    alloy_rpc_types::Transaction {
        inner: tx,
        from: signer,
        effective_gas_price: Some(effective_gas_price),
        block_hash,
        block_number,
        transaction_index,
    }
}
