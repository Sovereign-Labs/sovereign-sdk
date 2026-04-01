use std::ops::Range;

use alloy_consensus::proofs::{calculate_receipt_root, calculate_transaction_root};
use alloy_consensus::{
    serde_bincode_compat::Header as HeaderBincodeCompat,
    transaction::serde_bincode_compat::EthereumTxEnvelope as EthereumTxEnvelopeBincodeCompat,
    transaction::Recovered, Header,
};
use alloy_consensus::{EthereumTxEnvelope, TxEip4844, TxReceipt, EMPTY_ROOT_HASH};
use alloy_eips::{Encodable2718, Typed2718};
use alloy_primitives::private::alloy_rlp::Encodable;
use alloy_primitives::{Address, Sealable, Sealed, B256};
use alloy_primitives::{Bloom, TxHash};
use bytes::BufMut;
use derive_more::{Deref, DerefMut};
use derive_new::new;
use reth_ethereum_primitives::serde_bincode_compat::Receipt as ReceiptBincodeCompat;
use serde_with::serde_as;
use sov_modules_api::macros::UniversalWallet;
use sov_rollup_interface::da::Time;

const SYNTHETIC_BLOCK_HASH_PLACEHOLDER: &[u8; 32] =
    b"synthetic_block_hash\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";

pub fn synthetic_block_hash_for(block_number: u64, num_txs: u32) -> B256 {
    let mut hash = *SYNTHETIC_BLOCK_HASH_PLACEHOLDER;
    hash[20..28].copy_from_slice(&block_number.to_be_bytes());
    hash[28..32].copy_from_slice(&num_txs.to_be_bytes());
    hash.into()
}

#[cfg(feature = "native")]
pub fn is_synthetic_block_hash(hash: &B256) -> bool {
    hash.iter()
        .take(20)
        .eq(SYNTHETIC_BLOCK_HASH_PLACEHOLDER.iter().take(20))
}

#[cfg(feature = "native")]
pub fn parse_synthetic_block_hash(hash: &B256) -> Option<(u64, u32)> {
    if !is_synthetic_block_hash(hash) {
        return None;
    }
    let block_number = u64::from_be_bytes(hash[20..28].try_into().unwrap());
    let num_txs = u32::from_be_bytes(hash[28..32].try_into().unwrap());
    Some((block_number, num_txs))
}

/// Signed ethereum transaction
pub type TransactionSigned = EthereumTxEnvelope<TxEip4844>;
/// Block body with TransactionSigned
pub type BlockBody<T = TransactionSigned, H = Header> = alloy_consensus::BlockBody<T, H>;

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

impl Encodable2718 for TxSignedAndRecovered {
    fn encode_2718(&self, out: &mut dyn BufMut) {
        self.signed_transaction.encode_2718(out);
    }

    fn encode_2718_len(&self) -> usize {
        self.signed_transaction.encode_2718_len()
    }
}

impl Typed2718 for TxSignedAndRecovered {
    fn ty(&self) -> u8 {
        self.signed_transaction.ty()
    }
}

impl Encodable for TxSignedAndRecovered {
    fn encode(&self, out: &mut dyn BufMut) {
        self.signed_transaction.encode(out);
    }
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
        let body = BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: None,
        };
        alloy_consensus::Block::rlp_length_for(&self.header, &body)
    }
}

/// A synthetic block header without the roots (txs, receipts) and bloom, and gas used.
/// "synthetic" here means that the header has a fake hash which we can reverse engineer to identify which transaction are in the block.
#[derive(Debug, PartialEq, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct SyntheticBlockWithoutRootsAndBloom {
    /// The synthetic block header. All fields are set except for the roots (txs, receipts) and bloom, and gas used.
    /// Note that the state root is also synthetic;
    /// https://reth.rs/docs/reth_primitives/serde_bincode_compat/index.html
    header_without_roots_bloom_and_gas_used: Header,

    /// Transactions in this synthetic block.
    pub transactions: Range<u64>,
}

fn xor_hashes(a: B256, b: B256) -> B256 {
    let mut result = B256::ZERO;
    for ((a_byte, b_byte), result_byte) in a.iter().zip(b.iter()).zip(result.iter_mut()) {
        *result_byte = a_byte ^ b_byte;
    }
    result
}

impl SyntheticBlockWithoutRootsAndBloom {
    /// Creates a new synthetic block header.
    pub fn new(mut header: Header, transactions: Range<u64>) -> Self {
        if header.state_root == EMPTY_ROOT_HASH {
            header.state_root = xor_hashes(header.parent_hash, header.transactions_root);
        }
        Self {
            header_without_roots_bloom_and_gas_used: header,
            transactions,
        }
    }

    /// Returns the block number of the synthetic block.
    pub fn block_number(&self) -> u64 {
        self.header_without_roots_bloom_and_gas_used.number
    }

    /// Returns the synthetic block hash.
    pub fn hash(&self) -> B256 {
        synthetic_block_hash_for(
            self.header_without_roots_bloom_and_gas_used.number,
            (self.transactions.end - self.transactions.start)
                .try_into()
                .expect("Transactions range should be less than u32::MAX"),
        )
    }

    /// Returns the number of transactions in the synthetic block.
    pub fn num_transactions(&self) -> u64 {
        self.transactions.end - self.transactions.start
    }

    /// Returns the index of the first transaction in the synthetic block.
    pub fn first_tx_index(&self) -> u64 {
        self.transactions.start
    }

    /// Returns the index of the last transaction in the synthetic block. If the block is empty, returns the index of the last tx in the previous block.
    pub fn last_tx_index(&self) -> u64 {
        self.transactions.end.saturating_sub(1)
    }

    /// Returns the header of the synthetic block.
    pub fn partial_header(&self) -> &Header {
        &self.header_without_roots_bloom_and_gas_used
    }

    /// Finishes the synthetic block and seals it. Returns the sealed synthetic block and the transactions that were added to the block.
    /// This function is relatively heavy, since it computes the tx and receipts roots.
    ///
    /// We pass the transactions as an owned type and return it rather than using a reference since some reth helpers require constructing types
    /// with Vec<Tx>.
    pub fn finish_and_seal(
        mut self,
        transactions: Vec<TxSignedAndRecovered>,
        receipts: &[reth_primitives::Receipt],
    ) -> (SealedSynthetic, Vec<TxSignedAndRecovered>) {
        assert_eq!(
            transactions.len(),
            receipts.len(),
            "Transactions and receipts must have the same length"
        );
        let gas_used = receipts.last().map_or(0, |r| r.cumulative_gas_used);

        let hash = self.hash();
        let tx_root = calculate_transaction_root(transactions.as_slice());
        let receipts_root = calculate_receipt_root(receipts);
        let bloom = receipts
            .iter()
            .fold(Bloom::ZERO, |bloom, r| bloom | r.bloom());
        self.header_without_roots_bloom_and_gas_used.logs_bloom = bloom;
        self.header_without_roots_bloom_and_gas_used.gas_used = gas_used;
        self.header_without_roots_bloom_and_gas_used
            .transactions_root = tx_root;
        self.header_without_roots_bloom_and_gas_used.receipts_root = receipts_root;

        let body = BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: None,
        };
        let rlp_size = alloy_consensus::Block::rlp_length_for(
            &self.header_without_roots_bloom_and_gas_used,
            &body,
        );
        let txs = body.transactions;

        (
            SealedSynthetic {
                header: Sealed::new_unchecked(self.header_without_roots_bloom_and_gas_used, hash),
                transactions: self.transactions,
                rlp_size,
            },
            txs,
        )
    }
}

#[serde_as]
#[derive(Debug, PartialEq, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct SealedSynthetic {
    // The synthetic block header. Contains real bloom, tx root, and receipts root but, but the block hash will be fake.
    pub header: Sealed<Header>,
    /// The transactions in the synthetic block.
    pub transactions: Range<u64>,
    /// The RLP encoded size of the block (header + body).
    pub rlp_size: usize,
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
#[derive(Debug)]
pub enum MaybeSealedBlock {
    /// SealedBlock
    Sealed(SealedBlock),
    /// A synthetic block whose number matches the pending block number. It may be slightly older than the newest pending block
    PendingSynthetic(SyntheticBlockWithoutRootsAndBloom),
    /// A synthetic block whose number is different from the pending block number.
    /// This block is either in the past, or it doesn't exist yet
    PastSynthetic(SyntheticBlockWithoutRootsAndBloom),
}

#[cfg(feature = "native")]
impl MaybeSealedBlock {
    /// Hash of the block.
    pub fn hash(&self) -> Option<B256> {
        Some(match self {
            Self::Sealed(block) => block.header.hash(),
            Self::PendingSynthetic(block) => block.hash(),
            Self::PastSynthetic(block) => block.hash(),
        })
    }

    /// The block number.
    pub fn number(&self) -> u64 {
        match self {
            Self::Sealed(block) => block.header.number,
            Self::PendingSynthetic(block) => block.header_without_roots_bloom_and_gas_used.number,
            Self::PastSynthetic(block) => block.header_without_roots_bloom_and_gas_used.number,
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
            Self::PendingSynthetic(block) => block.transactions.start,
            Self::PastSynthetic(block) => block.transactions.start,
        }
    }

    /// Index of the last transaction in the block.
    pub fn transactions_end(&self) -> u64 {
        match self {
            Self::Sealed(block) => block.transactions.end,
            Self::PendingSynthetic(block) => block.transactions.end,
            Self::PastSynthetic(block) => block.transactions.end,
        }
    }

    /// The block timestamp.
    pub fn timestamp(&self) -> u64 {
        match self {
            Self::Sealed(block) => block.header.timestamp,
            Self::PendingSynthetic(block) => {
                block.header_without_roots_bloom_and_gas_used.timestamp
            }
            Self::PastSynthetic(block) => block.header_without_roots_bloom_and_gas_used.timestamp,
        }
    }

    /// The block header.
    pub fn maybe_partial_header(&self) -> &Header {
        match self {
            Self::Sealed(block) => block.header.inner(),
            Self::PendingSynthetic(block) => &block.header_without_roots_bloom_and_gas_used,
            Self::PastSynthetic(block) => &block.header_without_roots_bloom_and_gas_used,
        }
    }

    /// The block header.
    pub fn into_maybe_partial_header(self) -> Header {
        match self {
            Self::Sealed(block) => block.header.into_inner(),
            Self::PendingSynthetic(block) => block.header_without_roots_bloom_and_gas_used,
            Self::PastSynthetic(block) => block.header_without_roots_bloom_and_gas_used,
        }
    }

    /// Returns the header if the block has been sealed. Returns None for synthetic blocks.
    pub fn header_if_sealed(&self) -> Option<&Header> {
        match self {
            Self::Sealed(block) => Some(block.header.inner()),
            Self::PendingSynthetic(_) => None,
            Self::PastSynthetic(_) => None,
        }
    }
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

        let alloy_tx: Recovered<TransactionSigned> = tx.into();

        assert_eq!(signer, alloy_tx.signer());
    }

    #[test]
    fn test_synthetic_block_hash() {
        let hash = synthetic_block_hash_for(100, 10);
        assert!(is_synthetic_block_hash(&hash));
        assert_eq!(parse_synthetic_block_hash(&hash), Some((100, 10)));
        assert!(!is_synthetic_block_hash(&B256::ZERO));
    }
}
