use std::ops::Range;

use alloy_consensus::{serde_bincode_compat, Header};
use alloy_primitives::{Address, Sealable, Sealed, B256};
use reth_primitives::{TransactionSigned, TransactionSignedEcRecovered};
use revm::primitives::EVMError;
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

#[serde_as]
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct Block {
    /// Block header.
    #[serde_as(as = "serde_bincode_compat::Header")]
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
        s.serialize_field(
            "header",
            &serde_bincode_compat::Header::from(self.header.inner()),
        )?;
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
            #[serde_as(as = "serde_bincode_compat::Header")]
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
