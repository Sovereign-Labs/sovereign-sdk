use std::ops::Range;

use jsonrpsee::types::ErrorObject;
use reth_primitives::{Address, Header, SealedHeader, TransactionSigned, H256};
use revm::primitives::EVMError;
use thiserror::Error;

#[derive(serde::Deserialize, serde::Serialize, Debug, PartialEq, Clone)]
pub(crate) struct BlockEnv {
    pub(crate) number: u64,
    pub(crate) coinbase: Address,
    pub(crate) timestamp: u64,
    /// Prevrandao is used after Paris (aka TheMerge) instead of the difficulty value.
    pub(crate) prevrandao: H256,
    /// basefee is added in EIP1559 London upgrade
    pub(crate) basefee: u64,
    pub(crate) gas_limit: u64,
}

impl Default for BlockEnv {
    fn default() -> Self {
        Self {
            number: Default::default(),
            coinbase: Default::default(),
            timestamp: Default::default(),
            prevrandao: Default::default(),
            basefee: Default::default(),
            gas_limit: reth_primitives::constants::ETHEREUM_BLOCK_GAS_LIMIT,
        }
    }
}

/// Rlp encoded evm transaction.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct RlpEvmTransaction {
    /// Rlp data.
    pub rlp: Vec<u8>,
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TransactionSignedAndRecovered {
    /// Signer of the transaction
    pub(crate) signer: Address,
    /// Signed transaction
    pub(crate) signed_transaction: TransactionSigned,
    /// Block the transaction was added to
    pub(crate) block_number: u64,
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Block {
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

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct SealedBlock {
    /// Block header.
    pub(crate) header: SealedHeader,

    /// Transactions in this block.
    pub(crate) transactions: Range<u64>,
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Receipt {
    pub(crate) receipt: reth_primitives::Receipt,
    pub(crate) gas_used: u64,
    pub(crate) log_index_start: u64,
    pub(crate) error: Option<EVMError<u8>>,
}

#[derive(Error, Debug)]
pub enum RawEvmTxConversionError {
    #[error("Empty raw transaction data")]
    EmptyRawTransactionData,
    #[error("Failed to decode signed transaction")]
    FailedToDecodeSignedTransaction,
    #[error("Failed to recover signer")]
    FailedToRecoverSigner,
}

impl From<RawEvmTxConversionError> for jsonrpsee::core::Error {
    fn from(error: RawEvmTxConversionError) -> Self {
        jsonrpsee::core::Error::Call(ErrorObject::owned::<&[u8]>(
            jsonrpsee::types::error::INVALID_PARAMS_CODE,
            error.to_string(),
            None,
        ))
    }
}
