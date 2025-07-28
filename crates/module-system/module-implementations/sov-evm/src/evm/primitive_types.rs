use std::ops::Range;

use reth_primitives::revm_primitives::{Address, EVMError};
use reth_primitives::{Header, SealedHeader, TransactionSigned, TransactionSignedEcRecovered};
use sov_modules_api::macros::UniversalWallet;

/// RLP encoded evm transaction.
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    Debug,
    PartialEq,
    Eq,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    UniversalWallet,
)]
pub struct RlpEvmTransaction {
    /// Rlp data.
    pub rlp: Vec<u8>,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransactionSignedAndRecovered {
    /// Signer of the transaction
    pub(crate) signer: Address,
    /// Signed transaction
    pub(crate) signed_transaction: TransactionSigned,
    /// Block the transaction was added to
    pub(crate) block_number: u64,
}

impl TransactionSignedAndRecovered {
    /// The signed transaction that was recovered.
    pub fn signed_transaction(&self) -> &TransactionSigned {
        &self.signed_transaction
    }
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct Block {
    /// Block header.
    pub(crate) header: Header,

    /// Transactions in this block.
    pub(crate) transactions: Range<u64>,
}

impl Block {
    pub(crate) fn seal(self) -> SealedBlock {
        SealedBlock {
            header: self.header.seal_slow(),
            transactions: self.transactions,
        }
    }
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct SealedBlock {
    /// Block header.
    pub(crate) header: SealedHeader,

    /// Transactions in this block.
    pub(crate) transactions: Range<u64>,
}

impl SealedBlock {
    /// Returns the block header.
    pub fn header(&self) -> &SealedHeader {
        &self.header
    }

    /// Returns the block transactions.
    pub fn transactions(&self) -> &Range<u64> {
        &self.transactions
    }
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct Receipt {
    pub receipt: reth_primitives::Receipt,
    pub gas_used: u64,
    pub log_index_start: u64,
    pub error: Option<EVMError<u8>>,
}

impl From<TransactionSignedAndRecovered> for TransactionSignedEcRecovered {
    fn from(value: TransactionSignedAndRecovered) -> Self {
        TransactionSignedEcRecovered::from_signed_transaction(
            value.signed_transaction,
            value.signer,
        )
    }
}
