use alloy_consensus::Transaction;
use alloy_eips::eip2718::{Decodable2718, Eip2718Error};
use alloy_primitives::{Address, Bytes, U256};
use reth_primitives::{Recovered, TransactionSigned};
use reth_primitives_traits::SignedTransaction;
use revm::context::{BlockEnv, TransactionType, TxEnv};
use thiserror::Error;

use super::primitive_types::SealedBlock;
use crate::RlpEvmTransaction;

// BlockEnv from SealedBlock
impl From<SealedBlock> for BlockEnv {
    fn from(block: SealedBlock) -> Self {
        Self {
            number: U256::from(block.header.number),
            beneficiary: block.header.beneficiary,
            timestamp: U256::from(block.header.timestamp),
            prevrandao: Some(block.header.mix_hash),
            basefee: block.header.base_fee_per_gas.unwrap_or_default(),
            gas_limit: block.header.gas_limit,
            // Not used fields:
            blob_excess_gas_and_price: None,
            difficulty: Default::default(),
        }
    }
}

pub(crate) fn create_tx_env(nonce_to_use: u64, tx: &TransactionSigned, signer: Address) -> TxEnv {
    TxEnv {
        tx_type: TransactionType::Eip1559.into(),
        caller: signer,
        gas_limit: tx.gas_limit(),
        gas_price: tx.effective_gas_price(None),
        gas_priority_fee: tx.max_priority_fee_per_gas(),
        kind: tx.to().into(),
        value: tx.value(),
        data: tx.input().clone(),
        chain_id: tx.chain_id(),
        nonce: nonce_to_use,
        ..Default::default()
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
pub fn convert_to_transaction_signed(
    data: RlpEvmTransaction,
) -> Result<TransactionSigned, RlpConversionError> {
    let data = Bytes::from(data.rlp);
    // We can skip that, it is done inside `decode_enveloped` method
    if data.is_empty() {
        return Err(RlpConversionError::EmptyRawTx);
    }

    let tx = TransactionSigned::decode_2718(&mut data.as_ref())?;
    Ok(tx)
}

impl TryFrom<RlpEvmTransaction> for Recovered<TransactionSigned> {
    type Error = RlpConversionError;

    fn try_from(evm_tx: RlpEvmTransaction) -> Result<Self, Self::Error> {
        let tx: TransactionSigned = convert_to_transaction_signed(evm_tx)?;
        let tx = tx
            .try_into_recovered()
            .map_err(|_| RlpConversionError::InvalidSignature)?;

        Ok(tx)
    }
}
