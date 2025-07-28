use alloy_primitives::private::alloy_rlp::Error as RlpError;
use alloy_primitives::TxKind;
use reth_primitives::revm_primitives::{Address, BlockEnv, TxEnv, U256};
use reth_primitives::{Bytes as RethBytes, TransactionSigned, TransactionSignedEcRecovered};
use thiserror::Error;

use super::primitive_types::SealedBlock;
use crate::RlpEvmTransaction;

// BlockEnv from SealedBlock
impl From<SealedBlock> for BlockEnv {
    fn from(block: SealedBlock) -> Self {
        Self {
            number: U256::from(block.header.number),
            coinbase: block.header.beneficiary,
            timestamp: U256::from(block.header.timestamp),
            prevrandao: Some(block.header.mix_hash),
            basefee: block.header.base_fee_per_gas.map_or(U256::ZERO, U256::from),
            gas_limit: U256::from(block.header.gas_limit),
            // Not used fields:
            blob_excess_gas_and_price: None,
            difficulty: Default::default(),
        }
    }
}

pub(crate) fn create_tx_env(tx: &TransactionSigned, signer: Address) -> TxEnv {
    let to = match tx.to() {
        Some(addr) => TxKind::Call(addr),
        None => TxKind::Create,
    };

    TxEnv {
        caller: signer,
        gas_limit: tx.gas_limit(),
        gas_price: U256::from(tx.effective_gas_price(None)),
        gas_priority_fee: tx.max_priority_fee_per_gas().map(U256::from),
        transact_to: to,
        value: tx.value(),
        data: tx.input().clone(),
        chain_id: tx.chain_id(),
        nonce: Some(tx.nonce()),
        // TODO handle access list
        access_list: vec![],
        // EIP-4844 related fields
        blob_hashes: Default::default(),
        max_fee_per_blob_gas: None,
        // EIP-7702: TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1132
        authorization_list: None,
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
    DeserializationFailed(#[from] RlpError),
    /// Invalid signature during EC recovery
    #[error("Invalid signature")]
    InvalidSignature,
}

/// Coverts RLP encoded transaction to `TransactionSigned`.
pub fn convert_to_transaction_signed(
    data: RlpEvmTransaction,
) -> Result<TransactionSigned, RlpConversionError> {
    let data = RethBytes::from(data.rlp);
    // We can skip that, it is done inside `decode_enveloped` method
    if data.is_empty() {
        return Err(RlpConversionError::EmptyRawTx);
    }

    let tx = TransactionSigned::decode_enveloped(&mut data.as_ref())?;
    Ok(tx)
}

impl TryFrom<RlpEvmTransaction> for TransactionSignedEcRecovered {
    type Error = RlpConversionError;

    fn try_from(evm_tx: RlpEvmTransaction) -> Result<Self, Self::Error> {
        let tx = convert_to_transaction_signed(evm_tx)?;
        let tx = tx
            .into_ecrecovered()
            .ok_or(RlpConversionError::InvalidSignature)?;

        Ok(tx)
    }
}
