use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};

/// A cheaply cloneable bytes abstraction for use within the trust boundary of the node
/// (i.e. when interfacing with the database). Serializes and deserializes more efficiently,
/// than most bytes abstractions, but is vulnerable to out-of-memory attacks
/// when read from an untrusted source.
///
/// # Warning
/// Do not use this type when deserializing data from an untrusted source!!
#[derive(
    Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Default, BorshDeserialize, BorshSerialize,
)]
pub struct DbBytes(Arc<Vec<u8>>);
impl DbBytes {
    pub fn new(contents: Vec<u8>) -> Self {
        Self(Arc::new(contents))
    }
}

impl AsRef<[u8]> for DbBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

pub type DbHash = DbBytes;
pub type JmtValue = Option<Vec<u8>>;
pub(crate) type StateKey = Vec<u8>;

/// The on-disk format of a slot. Specifies the batches contained in the slot
/// and the hash of the da block. TODO(@preston-evans98): add any additional data
/// required to reconstruct the da block proof
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct StoredSlot {
    hash: DbHash,
    batches: std::ops::Range<BatchNumber>,
}

/// An identifier that specifies a single slot
#[derive(Debug, PartialEq)]
pub enum SlotIdentifier {
    Hash(DbHash),       // the hash of a da block
    Number(SlotNumber), // the blocknumber of a da block
}

/// The on-disk format for a batch. Stores the hash and identifies the range of transactions
/// included in the batch
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct StoredBatch {
    hash: DbBytes,
    txs: std::ops::Range<TxNumber>,
}

/// An identifier that specifies a single batch
#[derive(Debug, PartialEq)]
pub enum BatchIdentifier {
    Hash(DbHash),
    SlotIdAndIndex((SlotIdentifier, u64)),
    /// The monotonically increasing number of the batch, ordered by the DA layer For example, if the genesis slot
    /// contains 0 batches, slot 1 contains 2 txs, and slot 3 contains 3 txs,
    /// the last batch in block 3 would have number 5. The counter never resets.
    Number(BatchNumber),
}

/// An identifier that specifies a single transaction
#[derive(Debug, PartialEq)]
pub enum TxIdentifier {
    Hash(DbHash),
    BatchIdAndIndex((BatchIdentifier, u64)),
    /// The monotonically increasing number of the tx, ordered by the DA layer For example, if genesis
    /// contains 0 txs, batch 1 contains 8 txs, and batch 3 contains 7 txs,
    /// the last tx in batch 3 would have number 15. The counter never resets.
    Number(TxNumber),
}

/// The on-disk format of a transaction. Includes the txhash, the serialized tx data,
/// and identifies the events emitted by this transaction
#[derive(Debug, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct StoredTransaction {
    hash: DbHash,
    /// The range of event-numbers emitted by this transaction
    events: std::ops::Range<EventNumber>,
    data: DbBytes,
}

/// An identifier that specifies a single event
#[derive(Debug, PartialEq)]
pub enum EventIdentifier {
    TxIdAndIndex((TxIdentifier, u64)),
    TxIdAndKey((TxIdentifier, DbBytes)),
    /// The monotonically increasing number of the event, ordered by the DA layer For example, if the first tx
    /// contains 7 events, tx 2 contains 11 events, and tx 3 contains 7 txs,
    /// the last event in tx 3 would have number 25. The counter never resets.
    Number(EventNumber),
}

/// An identifier for a group of related events
#[derive(Debug, PartialEq)]
pub enum EventGroupIdentifier {
    TxId(TxIdentifier),
    Key(Vec<u8>),
}

macro_rules! u64_wrapper {
    ($name:ident) => {
        /// A typed wrapper around u64 implementing `Encode` and `Decode`
        #[derive(
            Clone,
            Copy,
            ::core::fmt::Debug,
            Default,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            ::borsh::BorshDeserialize,
            ::borsh::BorshSerialize,
        )]
        pub struct $name(pub u64);
    };
}

u64_wrapper!(SlotNumber);
u64_wrapper!(BatchNumber);
u64_wrapper!(TxNumber);
u64_wrapper!(EventNumber);
