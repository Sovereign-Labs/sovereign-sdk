use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::rpc::{BatchResponse, TxIdentifier, TxResponse};
use sov_rollup_interface::stf::{Event, EventKey, TransactionReceipt};

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

impl From<Vec<u8>> for DbBytes {
    fn from(value: Vec<u8>) -> Self {
        Self(Arc::new(value))
    }
}

#[derive(
    Debug, Clone, Copy, BorshDeserialize, BorshSerialize, PartialEq, Eq, Serialize, Deserialize,
)]
pub enum Status {
    Applied,
    Skipped,
    Reverted,
}

impl AsRef<[u8]> for DbBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

pub type DbHash = [u8; 32];
pub type JmtValue = Option<Vec<u8>>;
pub(crate) type StateKey = Vec<u8>;

/// The on-disk format of a slot. Specifies the batches contained in the slot
/// and the hash of the da block. TODO(@preston-evans98): add any additional data
/// required to reconstruct the da block proof
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct StoredSlot {
    pub hash: DbHash,
    pub extra_data: DbBytes,
    pub batches: std::ops::Range<BatchNumber>,
}

/// The on-disk format for a batch. Stores the hash and identifies the range of transactions
/// included in the batch
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct StoredBatch {
    pub hash: DbHash,
    pub txs: std::ops::Range<TxNumber>,
    pub custom_receipt: DbBytes,
}

impl<B: DeserializeOwned, T> TryFrom<StoredBatch> for BatchResponse<B, T> {
    type Error = anyhow::Error;
    fn try_from(value: StoredBatch) -> Result<Self, Self::Error> {
        Ok(Self {
            hash: value.hash,
            custom_receipt: bincode::deserialize(&value.custom_receipt.0)?,
            tx_range: value.txs.start.into()..value.txs.end.into(),
            txs: None,
        })
    }
}

/// The on-disk format of a transaction. Includes the txhash, the serialized tx data,
/// and identifies the events emitted by this transaction
#[derive(Debug, PartialEq, BorshSerialize, BorshDeserialize, Clone)]
pub struct StoredTransaction {
    pub hash: DbHash,
    /// The range of event-numbers emitted by this transaction
    pub events: std::ops::Range<EventNumber>,
    pub body: Option<Vec<u8>>,
    pub custom_receipt: DbBytes,
}

impl<R: DeserializeOwned> TryFrom<StoredTransaction> for TxResponse<R> {
    type Error = anyhow::Error;
    fn try_from(value: StoredTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            hash: value.hash,
            event_range: value.events.start.into()..value.events.end.into(),
            body: value.body,
            custom_receipt: bincode::deserialize(&value.custom_receipt.0)?,
        })
    }
}

pub fn split_tx_for_storage<R: Serialize>(
    tx: TransactionReceipt<R>,
    event_offset: u64,
) -> (StoredTransaction, Vec<Event>) {
    let event_range = EventNumber(event_offset)..EventNumber(event_offset + tx.events.len() as u64);
    let tx_for_storage = StoredTransaction {
        hash: tx.tx_hash,
        events: event_range,
        body: tx.body_to_save,
        custom_receipt: DbBytes::new(
            bincode::serialize(&tx.receipt).expect("Serialization to vec is infallible"),
        ),
    };
    (tx_for_storage, tx.events)
}

/// An identifier that specifies a single event
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum EventIdentifier {
    TxIdAndIndex((TxIdentifier, u64)),
    TxIdAndKey((TxIdentifier, EventKey)),
    /// The monotonically increasing number of the event, ordered by the DA layer For example, if the first tx
    /// contains 7 events, tx 2 contains 11 events, and tx 3 contains 7 txs,
    /// the last event in tx 3 would have number 25. The counter never resets.
    Number(EventNumber),
}

/// An identifier for a group of related events
#[derive(Debug, PartialEq, Serialize, Deserialize)]
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
            ::serde::Serialize,
            ::serde::Deserialize,
        )]
        pub struct $name(pub u64);

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

u64_wrapper!(SlotNumber);
u64_wrapper!(BatchNumber);
u64_wrapper!(TxNumber);
u64_wrapper!(EventNumber);
