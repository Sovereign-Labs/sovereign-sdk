use alloy_consensus::{transaction::Recovered, Transaction};
use alloy_eips::eip2718::{Decodable2718, Eip2718Error};
use alloy_primitives::{Address, Bytes, U256};
use reth_primitives_traits::SignedTransaction;
use revm::context::{BlockEnv, TransactionType, TxEnv};
use thiserror::Error;

use super::primitive_types::SealedBlock;
#[cfg(feature = "native")]
use crate::primitive_types::TransactionSignedAndRecovered;
use crate::{evm::primitive_types::TransactionSigned, RlpEvmTransaction};

// BlockEnv from SealedBlock
impl From<SealedBlock> for BlockEnv {
    fn from(block: SealedBlock) -> Self {
        Self {
            number: U256::from(block.header.number),
            beneficiary: block.header.beneficiary,
            timestamp: U256::from(block.header.timestamp),
            prevrandao: Some(block.header.mix_hash),
            basefee: 0,
            gas_limit: block.header.gas_limit,
            // Not used fields:
            blob_excess_gas_and_price: None,
            difficulty: Default::default(),
        }
    }
}

// Converts historical tx to TxEnv
#[cfg(feature = "native")]
pub fn replay_tx_env(tx: &TransactionSignedAndRecovered) -> TxEnv {
    let TransactionSignedAndRecovered {
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

        tx_type: TransactionType::Eip1559.into(),
        kind: tx.to().into(),
        value: tx.value(),
        data: tx.input().clone(),
        chain_id: tx.chain_id(),
        // We don't set gas_price nor the gas_priority_fee.
        // We disable the EVM logic charging gas at the beginning of the TX and instead rely on sov gas metering
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

#[cfg(test)]
mod tests {
    use crate::primitive_types::Block;

    use super::*;

    #[test]
    fn prepare_call_block_env() {
        let block = Block::default();
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
