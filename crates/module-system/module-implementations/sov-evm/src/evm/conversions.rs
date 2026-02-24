use alloy_consensus::transaction::{PooledTransaction, Recovered};
use alloy_consensus::Transaction;
use alloy_eips::eip2718::{Decodable2718, Eip2718Error};
use alloy_eips::Typed2718;
use alloy_primitives::{Address, Bytes, B256, U256};
use reth_primitives_traits::SignedTransaction;
use revm::{
    context::{BlockEnv, TxEnv},
    context_interface::block::BlobExcessGasAndPrice,
    context_interface::either::Either,
};
use thiserror::Error;

use super::primitive_types::SealedBlock;
#[cfg(feature = "native")]
use crate::primitive_types::TxSignedAndRecovered;
use crate::{
    evm::primitive_types::TransactionSigned, RlpEvmTransaction, SyntheticBlockWithoutRootsAndBloom,
    BLOB_GAS_PRICE, EXCESS_BLOB_GAS,
};

// BlockEnv from SealedBlock
impl From<SealedBlock> for BlockEnv {
    fn from(block: SealedBlock) -> Self {
        create_block_env(
            block.base_fee(),
            block.header.gas_limit,
            block.header.timestamp,
            block.header.beneficiary,
            block.header.number,
            Some(block.header.mix_hash),
        )
    }
}

impl From<SyntheticBlockWithoutRootsAndBloom> for BlockEnv {
    fn from(block: SyntheticBlockWithoutRootsAndBloom) -> Self {
        let header = block.partial_header();
        create_block_env(
            header
                .base_fee_per_gas
                .expect("Synthetic blocks have their base fee set"),
            header.gas_limit,
            header.timestamp,
            header.beneficiary,
            header.number,
            Some(header.mix_hash),
        )
    }
}

// Converts historical tx to TxEnv
#[cfg(feature = "native")]
pub fn replay_tx_env(tx: &TxSignedAndRecovered) -> TxEnv {
    let TxSignedAndRecovered {
        signed_transaction,
        signer,
        ..
    } = tx;
    create_tx_env(
        signed_transaction,
        *signer,
        signed_transaction.nonce(),
        signed_transaction.gas_limit(),
    )
}

/// Converts tx to TxEnv while overriding the signer and nonce
pub fn create_tx_env(tx: &TransactionSigned, signer: Address, nonce: u64, gas_limit: u64) -> TxEnv {
    TxEnv {
        caller: signer,
        gas_limit,
        nonce,
        tx_type: tx.ty(),
        kind: tx.to().into(),
        value: tx.value(),
        data: tx.input().clone(),
        chain_id: tx.chain_id(),
        // We disable fee charging in revm and charge via rollup metering, but tx fields still
        // need to match the signed transaction for opcode/RPC semantics.
        gas_price: tx.max_fee_per_gas(),
        access_list: tx.access_list().cloned().unwrap_or_default(),
        gas_priority_fee: tx.max_priority_fee_per_gas(),
        blob_hashes: tx
            .blob_versioned_hashes()
            .map(|hashes| hashes.to_vec())
            .unwrap_or_default(),
        max_fee_per_blob_gas: tx.max_fee_per_blob_gas().unwrap_or_default(),
        authorization_list: tx
            .authorization_list()
            .map(|auth| auth.iter().cloned().map(Either::Left).collect())
            .unwrap_or_default(),
    }
}

/// Error that happened during conversion between types
#[derive(Debug, Error)]
pub enum RlpConversionError {
    /// Raw transaction is empty.
    #[error("Empty raw transaction")]
    EmptyRawTx,
    /// Deserialization has failed.
    #[error("Deserialization failed")]
    DeserializationFailed(#[from] Eip2718Error),
    /// Invalid signature during EC recovery
    #[error("Invalid signature")]
    InvalidSignature,
}

/// Coverts RLP encoded transaction to `TransactionSigned`.
pub fn convert_to_tx_signed(
    data: RlpEvmTransaction,
) -> Result<TransactionSigned, RlpConversionError> {
    let data = Bytes::from(data.rlp);
    // We can skip that, it is done inside `decode_enveloped` method
    if data.is_empty() {
        return Err(RlpConversionError::EmptyRawTx);
    }

    // Decode both canonical tx envelopes and pooled blob envelopes (type-0x03 with sidecar),
    // stripping sidecar data from the latter to match internal consensus transaction storage.
    let tx = TransactionSigned::decode_2718_exact(data.as_ref())
        .or_else(|_| PooledTransaction::decode_2718_exact(data.as_ref()).map(Into::into))?;
    Ok(tx)
}

impl TryFrom<RlpEvmTransaction> for Recovered<TransactionSigned> {
    type Error = RlpConversionError;

    fn try_from(evm_tx: RlpEvmTransaction) -> Result<Self, Self::Error> {
        let tx: TransactionSigned = convert_to_tx_signed(evm_tx)?;
        let tx = tx
            .try_into_recovered()
            .map_err(|_| RlpConversionError::InvalidSignature)?;

        Ok(tx)
    }
}

pub(crate) fn create_block_env(
    base_fee: u64,
    gas_limit: u64,
    timestamp: u64,
    beneficiary: Address,
    number: u64,
    prevrandao: Option<B256>,
) -> BlockEnv {
    BlockEnv {
        number: U256::from(number),
        beneficiary,
        timestamp: U256::from(timestamp),
        prevrandao,
        gas_limit,
        basefee: base_fee,
        blob_excess_gas_and_price: Some(BlobExcessGasAndPrice {
            excess_blob_gas: EXCESS_BLOB_GAS,
            blob_gasprice: BLOB_GAS_PRICE,
        }),
        // Default values
        difficulty: U256::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use crate::primitive_types::Block;

    use super::*;

    #[test]
    fn prepare_call_block_env() {
        let mut block = Block::default();
        block.header.base_fee_per_gas = Some(0);

        let sealed_block = block.clone().seal();

        let block_env = BlockEnv::from(sealed_block);

        assert_eq!(block_env.number, block.header.number);
        assert_eq!(block_env.beneficiary, block.header.beneficiary);
        assert_eq!(block_env.timestamp, block.header.timestamp);
        assert_eq!(block_env.basefee, 0);
        assert_eq!(block_env.gas_limit, block.header.gas_limit);
        assert_eq!(block_env.prevrandao, Some(block.header.mix_hash));
    }
}
