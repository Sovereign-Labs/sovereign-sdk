use std::ops::Range;

use alloy_consensus::{
    serde_bincode_compat::Header as HeaderBincodeCompat,
    transaction::serde_bincode_compat::EthereumTxEnvelope as EthereumTxEnvelopeBincodeCompat,
    Header,
};
use alloy_primitives::{Address, Sealable, Sealed, B256};
use reth_ethereum_primitives::serde_bincode_compat::Receipt as ReceiptBincodeCompat;
use reth_primitives::{Recovered, TransactionSigned};
use revm::context::result::EVMError;
use serde_with::serde_as;
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

#[serde_as]
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransactionSignedAndRecovered {
    /// Signer of the transaction
    pub(crate) signer: Address,
    /// Signed transaction
    /// https://reth.rs/docs/reth_primitives/serde_bincode_compat/index.html
    #[serde_as(as = "EthereumTxEnvelopeBincodeCompat")]
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

/// A pending Ethereum transaction.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PendingTransaction {
    pub(crate) transaction: TransactionSignedAndRecovered,
    pub(crate) receipt: Receipt,
}

impl PendingTransaction {
    pub(crate) fn new(transaction: TransactionSignedAndRecovered, receipt: Receipt) -> Self {
        Self {
            transaction,
            receipt,
        }
    }
}

#[serde_as]
#[derive(Debug, PartialEq, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Block {
    /// Block header.
    /// https://reth.rs/docs/reth_primitives/serde_bincode_compat/index.html
    #[serde_as(as = "HeaderBincodeCompat")]
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

#[derive(Debug, PartialEq, Clone)]
pub struct SealedBlock {
    /// Block header.
    pub(crate) header: Sealed<Header>,

    /// Transactions in this block.
    pub(crate) transactions: Range<u64>,
}

impl SealedBlock {
    /// Returns the block header.
    pub fn header(&self) -> &Sealed<Header> {
        &self.header
    }

    /// Returns the block transactions.
    pub fn transactions(&self) -> &Range<u64> {
        &self.transactions
    }
}

impl serde::Serialize for SealedBlock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut s = serializer.serialize_struct("SealedBlock", 3)?;
        // serialize inner Header using bincode-compat wrapper
        s.serialize_field("header", &HeaderBincodeCompat::from(self.header.inner()))?;
        s.serialize_field("seal", &self.header.seal())?;
        s.serialize_field("transactions", &self.transactions)?;
        s.end()
    }
}

impl<'de> serde::Deserialize<'de> for SealedBlock {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[serde_as]
        #[derive(serde::Deserialize)]
        struct Raw {
            /// https://reth.rs/docs/reth_primitives/serde_bincode_compat/index.html
            #[serde_as(as = "HeaderBincodeCompat")]
            header: Header,
            seal: B256,
            transactions: Range<u64>,
        }

        let Raw {
            header,
            seal,
            transactions,
        } = Raw::deserialize(deserializer)?;
        Ok(SealedBlock {
            header: Sealed::new_unchecked(header, seal),
            transactions,
        })
    }
}

#[cfg(feature = "native")]
pub(crate) enum MaybeSealedBlock {
    Sealed(Box<SealedBlock>),
    Pending {
        block_number: u64,
        first_tx_number: u64,
        base_fee_per_gas: u64,
    },
}

#[cfg(feature = "native")]
impl MaybeSealedBlock {
    pub fn hash(&self) -> Option<B256> {
        match self {
            Self::Sealed(block) => Some(block.header.hash()),
            Self::Pending { .. } => None,
        }
    }

    pub fn number(&self) -> u64 {
        match self {
            Self::Sealed(block) => block.header.number,
            Self::Pending { block_number, .. } => *block_number,
        }
    }

    pub fn transactions_start(&self) -> u64 {
        match self {
            Self::Sealed(block) => block.transactions.start,
            Self::Pending {
                first_tx_number, ..
            } => *first_tx_number,
        }
    }

    pub fn timestamp(&self) -> Option<u64> {
        match self {
            Self::Sealed(block) => Some(block.header.timestamp),
            Self::Pending { .. } => None,
        }
    }

    pub fn base_fee_per_gas(&self) -> u64 {
        match self {
            Self::Sealed(block) => block
                .header
                .base_fee_per_gas
                .expect("Legacy blocks with no base fee are unsupported"),
            Self::Pending {
                base_fee_per_gas, ..
            } => *base_fee_per_gas,
        }
    }
}

#[serde_as]
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct Receipt {
    /// https://reth.rs/docs/reth_primitives/serde_bincode_compat/index.html
    #[serde_as(as = "ReceiptBincodeCompat")]
    pub receipt: reth_primitives::Receipt,
    pub gas_used: u64,
    pub log_index_start: u64,
    pub error: Option<EVMError<u8>>,
}

impl From<TransactionSignedAndRecovered> for Recovered<TransactionSigned> {
    fn from(value: TransactionSignedAndRecovered) -> Self {
        Recovered::new_unchecked(value.signed_transaction, value.signer)
    }
}

#[cfg(test)]
mod tests {
    use alloy_consensus::{EthereumTxEnvelope, Signed, TxEip1559};
    use alloy_primitives::Signature;

    use super::*;

    #[test]
    fn tx_conversion() {
        let signer = Address::random();
        let tx = TransactionSignedAndRecovered {
            signer,
            signed_transaction: EthereumTxEnvelope::Eip1559(Signed::new_unchecked(
                TxEip1559::default(),
                Signature::test_signature(),
                Default::default(),
            )),
            block_number: 5u64,
        };

        let reth_tx: Recovered<TransactionSigned> = tx.into();

        assert_eq!(signer, reth_tx.signer());
    }
}
