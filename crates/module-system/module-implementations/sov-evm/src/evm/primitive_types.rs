use std::ops::Range;

use alloy_consensus::{
    serde_bincode_compat::Header as HeaderBincodeCompat,
    transaction::serde_bincode_compat::EthereumTxEnvelope as EthereumTxEnvelopeBincodeCompat,
    transaction::Recovered, Header,
};
use alloy_consensus::{EthereumTxEnvelope, TxEip4844};
use alloy_primitives::TxHash;
use alloy_primitives::{Address, Sealable, Sealed, B256};
use derive_more::{Deref, DerefMut, From};
use derive_new::new;
use reth_ethereum_primitives::serde_bincode_compat::Receipt as ReceiptBincodeCompat;
use serde_with::serde_as;
use sov_modules_api::macros::UniversalWallet;
use sov_rollup_interface::da::Time;

/// Signed ethereum transaction
pub type TransactionSigned = EthereumTxEnvelope<TxEip4844>;

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
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize, Deref, DerefMut, new)]
pub struct TxSignedAndRecovered {
    /// Signer of the transaction
    pub(crate) signer: Address,
    /// Signed transaction
    /// https://reth.rs/docs/reth_primitives/serde_bincode_compat/index.html
    #[serde_as(as = "EthereumTxEnvelopeBincodeCompat")]
    #[deref]
    #[deref_mut]
    pub(crate) signed_transaction: TransactionSigned,
    /// Block the transaction was added to
    pub block_number: u64,
}

impl TxSignedAndRecovered {
    /// The signed transaction that was recovered.
    pub fn signed_transaction(&self) -> &TransactionSigned {
        &self.signed_transaction
    }
}

/// A pending Ethereum transaction.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PendingTransaction {
    pub(crate) transaction: TxSignedAndRecovered,
    pub(crate) receipt: Receipt,
    pub(crate) time: Time,
}

impl PendingTransaction {
    pub(crate) fn new(transaction: TxSignedAndRecovered, receipt: Receipt, time: Time) -> Self {
        Self {
            transaction,
            receipt,
            time,
        }
    }
}

#[serde_as]
#[derive(Debug, PartialEq, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Block {
    /// Block header.
    /// https://reth.rs/docs/reth_primitives/serde_bincode_compat/index.html
    #[serde_as(as = "HeaderBincodeCompat")]
    pub header: Header,

    /// Transactions in this block.
    pub transactions: Range<u64>,
}

impl Block {
    pub(crate) fn seal(self) -> SealedBlock {
        SealedBlock {
            header: self.header.seal_slow(),
            transactions: self.transactions,
            rlp_size: 0, // Will be set in finalize_hook when transactions are available
        }
    }

    #[cfg(feature = "native")]
    pub(crate) fn seal_with_size(self, transactions: Vec<TransactionSigned>) -> SealedBlock {
        let rlp_size = self.calculate_rlp_size(transactions);
        SealedBlock {
            header: self.header.seal_slow(),
            transactions: self.transactions,
            rlp_size,
        }
    }

    #[cfg(feature = "native")]
    fn calculate_rlp_size(&self, transactions: Vec<TransactionSigned>) -> usize {
        let body = reth_primitives::BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: None,
        };
        alloy_consensus::Block::rlp_length_for(&self.header, &body)
    }
}

/// Block with sealed header.
#[derive(Debug, PartialEq, Clone, Deref, DerefMut)]
pub struct SealedBlock {
    /// Block header.
    #[deref]
    #[deref_mut]
    pub header: Sealed<Header>,

    /// Transactions in this block.
    pub transactions: Range<u64>,

    /// RLP encoded size of the block (header + body).
    /// Cached to avoid re-fetching transactions for RPC queries.
    pub rlp_size: usize,
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

    /// EIP1559 base fee.
    pub fn base_fee(&self) -> u64 {
        self.header
            .base_fee_per_gas
            .expect("We set it at genesis and never remove it — only overwrite it")
    }
}

impl serde::Serialize for SealedBlock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut s = serializer.serialize_struct("SealedBlock", 4)?;
        // serialize inner Header using bincode-compat wrapper
        s.serialize_field("header", &HeaderBincodeCompat::from(self.header.inner()))?;
        s.serialize_field("seal", &self.header.seal())?;
        s.serialize_field("transactions", &self.transactions)?;
        s.serialize_field("rlp_size", &self.rlp_size)?;
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
            #[serde(default)]
            rlp_size: usize,
        }

        let Raw {
            header,
            seal,
            transactions,
            rlp_size,
        } = Raw::deserialize(deserializer)?;
        Ok(SealedBlock {
            header: Sealed::new_unchecked(header, seal),
            transactions,
            rlp_size,
        })
    }
}

#[cfg(feature = "native")]
/// Sealed or pending block.
#[derive(From, Debug)]
pub enum MaybeSealedBlock {
    /// SealedBlock
    Sealed(SealedBlock),
    /// Pending
    Pending(crate::Block),
}

#[cfg(feature = "native")]
impl MaybeSealedBlock {
    /// Hash of the block.
    pub fn hash(&self) -> Option<B256> {
        match self {
            Self::Sealed(block) => Some(block.header.hash()),
            // Pending blocks are not sealed yet, but we still expose a deterministic synthetic hash
            // so RPC clients can correlate receipts with "latest" block queries.
            Self::Pending(pending) => Some(pending_block_hash(pending)),
        }
    }

    /// The block number.
    pub fn number(&self) -> u64 {
        match self {
            Self::Sealed(block) => block.header.number,
            Self::Pending(pending) => pending.header.number,
        }
    }

    /// The range of transactions in the block
    pub fn tx_range(&self) -> Range<u64> {
        self.transactions_start()..self.transactions_end()
    }

    /// Index of the first transaction in the block.
    pub fn transactions_start(&self) -> u64 {
        match self {
            Self::Sealed(block) => block.transactions.start,
            Self::Pending(pending) => pending.transactions.start,
        }
    }

    /// Index of the last transaction in the block.
    pub fn transactions_end(&self) -> u64 {
        match self {
            Self::Sealed(block) => block.transactions.end,
            Self::Pending(pending) => pending.transactions.end,
        }
    }

    /// The block timestamp.
    pub fn timestamp(&self) -> u64 {
        match self {
            Self::Sealed(block) => block.header.timestamp,
            Self::Pending(pending) => pending.header.timestamp,
        }
    }

    /// The block header.
    pub fn header(&self) -> &Header {
        match self {
            Self::Sealed(block) => block.header.inner(),
            Self::Pending(pending) => &pending.header,
        }
    }

    /// The block header.
    pub fn into_header(self) -> Header {
        match self {
            Self::Sealed(block) => block.header.into_inner(),
            Self::Pending(pending) => pending.header,
        }
    }
}

#[cfg(feature = "native")]
impl From<MaybeSealedBlock> for Sealed<Header> {
    fn from(block: MaybeSealedBlock) -> Sealed<Header> {
        let hash = block.hash().unwrap_or_default();
        let header = block.into_header();
        Sealed::new_unchecked(header, hash)
    }
}

pub(crate) fn pending_block_hash(block: &Block) -> B256 {
    // We expose a synthetic hash for pending blocks because receipts are returned before the
    // block is sealed. The hash encodes `[block_number || last_tx_index]` so:
    // - it stays stable for a given pending snapshot
    // - it changes as new txs are appended
    //
    // Note: the tx range is half-open (start..end), so the last valid tx index is end-1.
    let tx_index = if block.transactions.start < block.transactions.end {
        block.transactions.end.saturating_sub(1)
    } else {
        block.transactions.end
    };
    synthetic_block_hash(block.header.number, tx_index)
}

pub(crate) fn synthetic_block_hash(block_number: u64, tx_index: u64) -> B256 {
    // Encode `[block_number || tx_index]` into the lower 16 bytes of the hash.
    // The higher 16 bytes stay zero for easy decode in RPC lookups.
    let mut bytes = [0u8; 32];
    bytes[16..24].copy_from_slice(&block_number.to_be_bytes());
    bytes[24..32].copy_from_slice(&tx_index.to_be_bytes());
    B256::from(bytes)
}

pub(crate) fn decode_synthetic_block_hash(hash: B256) -> (u64, u64) {
    // Decode the `[block_number || tx_index]` synthetic hash format.
    let bytes: &[u8; 32] = hash.as_ref();
    let mut block_number = [0u8; 8];
    let mut tx_index = [0u8; 8];
    block_number.copy_from_slice(&bytes[16..24]);
    tx_index.copy_from_slice(&bytes[24..32]);
    (
        u64::from_be_bytes(block_number),
        u64::from_be_bytes(tx_index),
    )
}

/// TODO: Can we replace this with Reth type?
#[serde_as]
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize, Deref, DerefMut)]
/// Receipt
pub struct Receipt {
    /// https://reth.rs/docs/reth_primitives/serde_bincode_compat/index.html
    #[serde_as(as = "ReceiptBincodeCompat")]
    #[deref]
    #[deref_mut]
    pub receipt: reth_primitives::Receipt,
    /// tx hash
    pub transaction_hash: TxHash,
    /// tx index
    pub transaction_index: u64,
    /// block number
    pub block_number: u64,
    /// gas used
    pub gas_used: u64,
    /// log index start
    pub log_index_start: u64,
}

impl From<TxSignedAndRecovered> for Recovered<TransactionSigned> {
    fn from(value: TxSignedAndRecovered) -> Self {
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
        let tx = TxSignedAndRecovered {
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
