use alloy_eips::eip2930::AccessList;
use alloy_primitives::{TxKind as PrimitiveTransactionKind, TxKind};
use alloy_rpc_types::AccessListItem;
use reth_primitives::revm_primitives::{BlockEnv, TxEnv, B256, U256};
use reth_primitives::{
    BlockNumber, Transaction as PrimitiveTransaction, TransactionSignedEcRecovered, TxType,
};
use reth_rpc_eth_types::revm_utils::CallFees;
use reth_rpc_eth_types::{EthResult, RpcInvalidTransactionError};
use reth_rpc_types::{Header, Parity, Signature, TransactionRequest};

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
        gas_limit: gas_limit
            .try_into()
            .map_err(|_| RpcInvalidTransactionError::GasUintOverflow)?,
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

pub(crate) fn from_primitive_with_hash(primitive_header: reth_primitives::SealedHeader) -> Header {
    let (header, hash) = primitive_header.split();
    let reth_primitives::Header {
        parent_hash,
        ommers_hash,
        beneficiary,
        state_root,
        transactions_root,
        receipts_root,
        logs_bloom,
        difficulty,
        number,
        gas_limit,
        gas_used,
        timestamp,
        mix_hash,
        nonce,
        base_fee_per_gas,
        requests_root,
        extra_data,
        withdrawals_root,
        blob_gas_used,
        excess_blob_gas,
        parent_beacon_block_root,
    } = header;

    Header {
        hash: Some(hash),
        parent_hash,
        uncles_hash: ommers_hash,
        miner: beneficiary,
        state_root,
        transactions_root,
        receipts_root,
        withdrawals_root,
        number: Some(number),
        gas_used: gas_used as u128,
        gas_limit: gas_limit as u128,
        extra_data,
        logs_bloom,
        timestamp,
        difficulty,
        mix_hash: Some(mix_hash),
        nonce: Some(nonce.to_be_bytes().into()),
        base_fee_per_gas: base_fee_per_gas.map(u128::from),
        blob_gas_used: blob_gas_used.map(u128::from),
        excess_blob_gas: excess_blob_gas.map(u128::from),
        parent_beacon_block_root,
        total_difficulty: None,
        requests_root,
    }
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
    let mut signed_tx = tx.into_signed();

    let to = match signed_tx.kind() {
        PrimitiveTransactionKind::Create => None,
        PrimitiveTransactionKind::Call(to) => Some(reth_primitives::Address(*to)),
    };

    let (gas_price, max_fee_per_gas) = match signed_tx.tx_type() {
        TxType::Legacy | TxType::Eip2930 => (Some(signed_tx.max_fee_per_gas()), None),
        TxType::Eip1559 => {
            // the gas price field for EIP-1559 is set to
            // `min(tip, gasFeeCap - baseFee) + baseFee`
            let gas_price = base_fee
                .and_then(|base_fee| {
                    signed_tx
                        .effective_tip_per_gas(Some(base_fee))
                        .map(|tip| tip + base_fee as u128)
                })
                .unwrap_or_else(|| signed_tx.max_fee_per_gas());

            (Some(gas_price), Some(signed_tx.max_fee_per_gas()))
        }
        TxType::Eip4844 => {
            panic!("EIP-4844 transactions are not supported by the rollup")
        }
        TxType::Eip7702 => {
            // TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1132
            panic!("EIP-7702 transactions are not yet supported by the rollup")
        }
    };

    let chain_id = signed_tx.chain_id();

    let access_list: Option<Vec<AccessListItem>> = match &mut signed_tx.transaction {
        PrimitiveTransaction::Legacy(_) => None,
        PrimitiveTransaction::Eip2930(tx) => Some(
            tx.access_list
                .0
                .iter()
                .map(|item| AccessListItem {
                    address: item.address.0.into(),
                    storage_keys: item.storage_keys.iter().map(|key| key.0.into()).collect(),
                })
                .collect(),
        ),
        PrimitiveTransaction::Eip1559(tx) => Some(
            tx.access_list
                .0
                .iter()
                .map(|item| AccessListItem {
                    address: item.address.0.into(),
                    storage_keys: item.storage_keys.iter().map(|key| key.0.into()).collect(),
                })
                .collect(),
        ),
        PrimitiveTransaction::Eip4844(_tx) => {
            panic!("EIP-4844 transactions are not supported by the rollup");
        }
        PrimitiveTransaction::Eip7702(_tx) => {
            // TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1132
            panic!("EIP-7702 transactions are not yet supported by the rollup")
        }
    };

    let signature = from_primitive_signature(
        *signed_tx.signature(),
        signed_tx.tx_type(),
        signed_tx.chain_id(),
    );

    alloy_rpc_types::Transaction {
        hash: signed_tx.hash(),
        nonce: signed_tx.nonce(),
        from: signer,
        to,
        value: signed_tx.value(),
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas: signed_tx.max_priority_fee_per_gas(),
        signature: Some(signature),
        gas: u128::from(signed_tx.gas_limit()),
        input: signed_tx.input().clone(),
        chain_id,
        access_list: access_list.map(AccessList::from),
        transaction_type: Some(signed_tx.tx_type() as u8),

        // These fields are set to None because they are not stored as part of the transaction
        block_hash,
        block_number,
        transaction_index,
        // EIP-4844 fields
        max_fee_per_blob_gas: Default::default(),
        blob_versioned_hashes: Default::default(),
        // Other fields
        other: Default::default(),
        // EIP-7702: TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1132
        authorization_list: None,
    }
}

pub(crate) fn from_primitive_signature(
    signature: reth_primitives::Signature,
    tx_type: TxType,
    chain_id: Option<u64>,
) -> Signature {
    match tx_type {
        TxType::Legacy => Signature {
            r: signature.r,
            s: signature.s,
            v: U256::from(signature.v(chain_id)),
            y_parity: None,
        },
        _ => Signature {
            r: signature.r,
            s: signature.s,
            v: U256::from(signature.odd_y_parity as u8),
            y_parity: Some(Parity(signature.odd_y_parity)),
        },
    }
}
