use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::Time;
use sov_rollup_interface::node::ledger_api::{BatchResponse, TxResponse};
use sov_rollup_interface::stf::{StoredEvent, TransactionReceipt, TxReceiptContents};

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
#[cfg_attr(
    feature = "arbitrary",
    derive(proptest_derive::Arbitrary, arbitrary::Arbitrary)
)]
pub struct DbBytes(Arc<Vec<u8>>);

impl DbBytes {
    /// Create `DbBytes` from a `Vec<u8>`
    pub fn new(contents: Vec<u8>) -> Self {
        Self(Arc::new(contents))
    }
}

impl From<Vec<u8>> for DbBytes {
    fn from(value: Vec<u8>) -> Self {
        Self(Arc::new(value))
    }
}

impl AsRef<[u8]> for DbBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// The "key" half of a key/value pair from accessory state.
///
/// See [`AccessoryDb`](crate::accessory_db::AccessoryDb) for more information.
pub type AccessoryKey = Vec<u8>;
/// The "value" half of a key/value pair from accessory state.
///
/// See [`AccessoryDb`](crate::accessory_db::AccessoryDb) for more information.
pub type AccessoryStateValue = Option<Vec<u8>>;

/// A hash stored in the database
pub type DbHash = [u8; 32];
/// The "value" half of a key/value pair from the JMT
pub type JmtValue = Option<Vec<u8>>;

/// The on-disk format of a slot. Specifies the batches contained in the slot
/// and the hash of the da block. TODO(@preston-evans98): add any additional data
/// required to reconstruct the da block proof.
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
#[cfg_attr(
    feature = "arbitrary",
    derive(proptest_derive::Arbitrary, ::arbitrary::Arbitrary)
)]
pub struct StoredSlot {
    /// The slot's hash, as reported by the DA layer.
    pub hash: DbHash,
    /// The root hash of the slot's JMT state.
    pub state_root: DbBytes,
    /// Any extra data which the rollup decides to store relating to this slot.
    pub extra_data: DbBytes,
    /// The range of batches which occurred in this slot.
    pub batches: std::ops::Range<BatchNumber>,
    /// The timestamp of the slot.
    pub timestamp: Time,
}

/// The on-disc format for information about state transition.
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct StoredStfInfo {
    /// The serialized StateTransitionInfo structure.
    pub data: Vec<u8>,
}

/// The on-disk format for a discarded blob.
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct StoredDiscardedBlob {
    /// The blob hash.
    pub hash: DbHash,
    /// This blobs's parent slot number.
    pub slot_number: SlotNumber,
}

/// The on-disk format for a batch. Stores the hash and identifies the range of transactions
/// included in the batch.
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
#[cfg_attr(
    feature = "arbitrary",
    derive(proptest_derive::Arbitrary, arbitrary::Arbitrary)
)]
pub struct StoredBatch {
    /// The hash of the batch, as reported by the DA layer.
    pub hash: DbHash,
    /// The range of transactions which occurred in this batch.
    pub txs: std::ops::Range<TxNumber>,
    /// A customer "receipt" for this batch defined by the rollup.
    pub receipt: DbBytes,
    /// This batch's parent slot number.
    pub slot_number: SlotNumber,
}

impl StoredBatch {
    /// Converts [`StoredBatch`] to [`BatchResponse`]
    pub fn to_batch_response<B: DeserializeOwned, T: TxReceiptContents, E>(
        &self,
    ) -> Result<BatchResponse<B, T, E>, anyhow::Error> {
        Ok(BatchResponse {
            hash: self.hash,
            receipt: bincode::deserialize(&self.receipt.0)?,
            tx_range: self.txs.start.into()..self.txs.end.into(),
            txs: None,
            slot_number: self.slot_number,
        })
    }
}

/// The on-disk format of a transaction. Includes the txhash, the serialized tx data,
/// and identifies the events emitted by this transaction
#[derive(Debug, PartialEq, BorshSerialize, BorshDeserialize, Clone)]
#[cfg_attr(
    feature = "arbitrary",
    derive(proptest_derive::Arbitrary, arbitrary::Arbitrary)
)]
pub struct StoredTransaction {
    /// The hash of the transaction.
    pub hash: DbHash,
    /// The range of event-numbers emitted by this transaction.
    pub events: std::ops::Range<EventNumber>,
    /// The serialized transaction data, if the rollup decides to store it.
    pub body: Option<Vec<u8>>,
    /// A custom "receipt" for this transaction defined by the rollup.
    pub receipt: DbBytes,
    /// This transaction's parent batch number.
    pub batch_number: BatchNumber,
}

impl<R: TxReceiptContents, E> TryFrom<StoredTransaction> for TxResponse<R, E> {
    type Error = anyhow::Error;
    fn try_from(value: StoredTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            hash: value.hash,
            event_range: value.events.start.into()..value.events.end.into(),
            body: value.body,
            events: None,
            receipt: bincode::deserialize(&value.receipt.0)?,
            batch_number: value.batch_number.0,
        })
    }
}

/// Split a [`TransactionReceipt`] into a [`StoredTransaction`] and a list of
/// [`StoredEvent`]s for storage in the database.
pub fn split_tx_for_storage<T: TxReceiptContents>(
    tx: TransactionReceipt<T>,
    batch_number: BatchNumber,
    event_offset: u64,
) -> (StoredTransaction, Vec<StoredEvent>) {
    let event_range =
        EventNumber(event_offset)..EventNumber(event_offset.saturating_add(tx.events.len() as u64));
    let tx_for_storage = StoredTransaction {
        hash: tx.tx_hash.into(),
        events: event_range,
        body: tx.body_to_save,
        receipt: DbBytes::new(
            bincode::serialize(&tx.receipt).expect("Serialization to vec is infallible"),
        ),
        batch_number,
    };

    let events_with_tx_hash = tx
        .events
        .into_iter()
        .map(|event| {
            StoredEvent::new(
                event.key().inner(),
                event.value().inner(),
                tx.tx_hash.into(),
            )
        })
        .collect();

    (tx_for_storage, events_with_tx_hash)
}

/// A singleton key for the latest finalized slot
#[derive(
    Clone,
    Copy,
    ::core::fmt::Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
pub struct LatestFinalizedSlotSingleton;

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
        #[cfg_attr(
            feature = "arbitrary",
            derive(proptest_derive::Arbitrary, arbitrary::Arbitrary)
        )]
        pub struct $name(pub u64);

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl ::core::ops::Add<u64> for $name {
            type Output = Self;

            fn add(self, rhs: u64) -> Self {
                Self(self.0 + rhs)
            }
        }

        impl ::core::ops::AddAssign<u64> for $name {
            fn add_assign(&mut self, rhs: u64) {
                self.0 += rhs;
            }
        }

        impl ::core::ops::Sub<u64> for $name {
            type Output = Self;

            fn sub(self, rhs: u64) -> Self {
                Self(self.0 - rhs)
            }
        }

        impl ::core::ops::SubAssign<u64> for $name {
            fn sub_assign(&mut self, rhs: u64) {
                self.0 -= rhs;
            }
        }
    };
}

u64_wrapper!(TxIncrId);
u64_wrapper!(BatchNumber);
u64_wrapper!(TxNumber);
u64_wrapper!(EventNumber);
u64_wrapper!(ProofUniqueId);
u64_wrapper!(StfInfoUniqueId);
u64_wrapper!(StateRootHashId);
