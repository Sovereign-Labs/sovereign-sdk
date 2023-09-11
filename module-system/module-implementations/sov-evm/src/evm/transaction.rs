use std::ops::Range;

use reth_primitives::{Address, SealedHeader, TransactionSigned, H256};

#[derive(serde::Deserialize, serde::Serialize, Debug, PartialEq, Clone)]
pub(crate) struct BlockEnv {
    pub(crate) number: u64,
    pub(crate) coinbase: Address,
    pub(crate) timestamp: u64,
    /// Prevrandao is used after Paris (aka TheMerge) instead of the difficulty value.
    pub(crate) prevrandao: Option<H256>,
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
            prevrandao: Some(Default::default()),
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
pub struct TransactionSignedAndRecovered {
    /// Signer of the transaction
    pub signer: Address,
    /// Signed transaction
    pub signed_transaction: TransactionSigned,
    /// Block the transaction was added to
    pub block_number: u64,
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(Debug, PartialEq, Clone)]
pub struct Block {
    /// Block header.
    pub header: SealedHeader,

    /// Transactions in this block.
    pub transactions: Range<u64>,
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(Debug, PartialEq, Clone)]
pub struct Receipt {
    pub receipt: reth_primitives::Receipt,
    pub gas_used: u64,
    pub log_index_start: u64,
}
