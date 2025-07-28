//! The rpc module defines types and traits for querying chain history
//! via an RPC interface.
use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use derive_more::derive::Display;
use futures::stream::BoxStream;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::common::{hex_string_serde, SlotNumber};
use crate::da::Time;
use crate::stf::{EventKey, StoredEvent, TxEffect, TxReceiptContents};
use crate::zk::aggregated_proof::SerializedAggregatedProof;

/// The finality status of a slot.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(proptest_derive::Arbitrary, arbitrary::Arbitrary)
)]
#[serde(rename_all = "snake_case")]
pub enum FinalityStatus {
    /// The slot has been produced but not finalized by consensus.
    Pending,
    /// The slot has been finalized.
    Finalized,
}

/// A struct containing enough information to uniquely specify single batch.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotIdAndOffset {
    /// The [`SlotIdentifier`] of the slot containing this batch.
    pub slot_id: SlotIdentifier,
    /// The offset into the slot at which this tx is located.
    /// Index 0 is the first batch in the slot.
    pub offset: u64,
}

/// A struct containing enough information to uniquely specify single transaction.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchIdAndOffset {
    /// The [`BatchIdentifier`] of the batch containing this transaction.
    pub batch_id: BatchIdentifier,
    /// The offset into the batch at which this tx is located.
    /// Index 0 is the first transaction in the batch.
    pub offset: u64,
}

/// A struct containing enough information to uniquely specify single event.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct TxIdAndOffset {
    /// The [`TxIdentifier`] of the transaction containing this event.
    pub tx_id: TxIdentifier,
    /// The offset into the tx's events at which this event is located.
    /// Index 0 is the first event from this tx.
    pub offset: u64,
}

/// An identifier that specifies a single batch
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum BatchIdentifier {
    /// The hex-encoded hash of the batch, as computed by the DA layer.
    Hash(#[serde(with = "hex_string_serde")] [u8; 32]),
    /// An offset into a particular slot (i.e. the 3rd batch in slot 5).
    SlotIdAndOffset(SlotIdAndOffset),
    /// The monotonically increasing number of the batch, ordered by the DA layer For example, if the genesis slot
    /// contains 0 batches, slot 1 contains 2 txs, and slot 3 contains 3 txs,
    /// the last batch in block 3 would have number 5. The counter never resets.
    Number(u64),
}

/// An identifier that specifies a single transaction.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum TxIdentifier {
    /// The hex encoded hash of the transaction.
    Hash(#[serde(with = "hex_string_serde")] [u8; 32]),
    /// An offset into a particular batch (i.e. the 3rd transaction in batch 5).
    BatchIdAndOffset(BatchIdAndOffset),
    /// The monotonically increasing number of the tx, ordered by the DA layer For example, if genesis
    /// contains 0 txs, batch 1 contains 8 txs, and batch 3 contains 7 txs,
    /// the last tx in batch 3 would have number 15. The counter never resets.
    Number(u64),
}

/// A struct containing enough information to uniquely specify single event.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct TxIdAndKey {
    /// The [`TxIdentifier`] of the transaction containing this event.
    pub tx_id: TxIdentifier,
    /// The key of the event.
    pub key: EventKey,
}

/// An identifier that specifies a single event.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum EventIdentifier {
    /// An offset into a particular transaction (i.e. the 3rd event in transaction number 5).
    TxIdAndOffset(TxIdAndOffset),
    /// The monotonically increasing number of the event, ordered by the DA layer For example, if the first tx
    /// contains 7 events, tx 2 contains 11 events, and tx 3 contains 7 events,
    /// the last event in tx 3 would have number 25. The counter never resets.
    Number(u64),
}

/// An identifier for a group of related events
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum EventGroupIdentifier {
    /// Fetch all events from a particular transaction.
    TxId(TxIdentifier),
    /// All events which occurred in a particular transaction with a particular key.
    TxIdAndKey(TxIdAndKey),
    /// Fetch all events (i.e. from all transactions) with a particular key.
    Key(EventKey),
}

/// An identifier that specifies a single slot.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum SlotIdentifier {
    /// The hex encoded hash of the slot (i.e. the da layer's block hash).
    Hash(#[serde(with = "hex_string_serde")] [u8; 32]),
    /// The monotonically increasing number of the slot, ordered by the DA layer but starting from 0
    /// at the *rollup's* genesis.
    Number(SlotNumber),
}

/// A QueryMode specifies how much information to return in response to an RPC query
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum QueryMode {
    /// Returns the parent struct but no details about its children.
    /// For example, a `Compact` "get_slots" response would simply state the range of batch
    /// numbers which occurred in the slot, but not the hashes of the batches themselves.
    Compact,
    /// Returns the parent struct and the hashes of all its children.
    Standard,
    /// Returns the parent struct and all its children, recursively fetching its children
    /// in `Full` mode. For example, a `Full` "get_batch" response would include the `Full`
    /// details of all the transactions in the batch, and those would in turn return the event bodies
    /// which had occurred in those transactions.
    Full,
}

impl Default for QueryMode {
    fn default() -> Self {
        Self::Standard
    }
}

/// [`IncludeChildren`] is used as a query parameter for [`QueryMode`] inside the ledger-api
#[derive(
    Debug, Copy, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, Display,
)]
#[display("{}", self.children)]
pub struct IncludeChildren {
    children: u8,
}

impl IncludeChildren {
    /// Creates a new [`IncludeChildren`] struct.
    pub fn new(include: bool) -> Self {
        if include {
            Self { children: 1 }
        } else {
            Self { children: 0 }
        }
    }

    fn includes_children(&self) -> bool {
        self.children != 0
    }
}

impl From<IncludeChildren> for QueryMode {
    fn from(value: IncludeChildren) -> Self {
        if value.includes_children() {
            QueryMode::Full
        } else {
            QueryMode::Compact
        }
    }
}

/// The body of a response to a JSON-RPC request for a particular slot.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(
    bound = "B: Serialize + DeserializeOwned, Tx: TxReceiptContents, E: Serialize + DeserializeOwned"
)]
pub struct SlotResponse<B, Tx: TxReceiptContents, E> {
    /// The rollup height.
    pub number: u64,
    /// The hex encoded slot hash.
    #[serde(with = "hex_string_serde")]
    pub hash: [u8; 32],
    /// The hex encoded state root hash.
    #[serde(with = "hex_string_serde")]
    pub state_root: Vec<u8>,
    /// The range of batches in this slot.
    pub batch_range: core::ops::Range<u64>,
    /// The batches in this slot, if the [`QueryMode`] of the request is not `Compact`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batches: Option<Vec<ItemOrHash<BatchResponse<B, Tx, E>>>>,
    /// The status of the slot.
    pub finality_status: FinalityStatus,
    /// The timestamp of the slot.
    pub timestamp: Time,
}

/// The response to a JSON-RPC request for a particular batch.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(
    bound = "B: Serialize + DeserializeOwned, Tx: TxReceiptContents, E: Serialize + DeserializeOwned"
)]
pub struct BatchResponse<B, Tx: TxReceiptContents, E> {
    /// The hex encoded batch hash.
    #[serde(with = "hex_string_serde")]
    pub hash: [u8; 32],
    /// The range of transactions in this batch.
    pub tx_range: core::ops::Range<u64>,
    /// The transactions in this batch, if the [`QueryMode`] of the request is not `Compact`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub txs: Option<Vec<ItemOrHash<TxResponse<Tx, E>>>>,
    /// The custom receipt specified by the rollup. This typically contains
    /// information about the outcome of the batch.
    pub receipt: B,
    /// The rollup height this batch belongs to.
    pub slot_number: SlotNumber,
}

/// The response to a JSON-RPC request for a particular transaction.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(bound = "Tx: TxReceiptContents, E: Serialize + DeserializeOwned")]
pub struct TxResponse<Tx: TxReceiptContents, E> {
    /// The hex encoded transaction hash.
    #[serde(with = "hex_string_serde")]
    pub hash: [u8; 32],
    /// The range of events occurring in this transaction.
    pub event_range: core::ops::Range<u64>,
    /// The transaction body, if stored by the rollup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Vec<u8>>,
    /// The events emitted by this transaction, if the [`QueryMode`] of the request is not `Compact`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<E>>,
    /// The custom receipt specified by the rollup. This typically contains
    /// information about the outcome of the transaction.
    pub receipt: TxEffect<Tx>,
    /// The batch number this transaction belongs to.
    pub batch_number: u64,
}

/// An RPC response which might contain a full item or just its hash.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum ItemOrHash<T> {
    /// The hex encoded hash of the requested item.
    Hash(#[serde(with = "hex_string_serde")] [u8; 32]),
    /// The full item body.
    Full(T),
}

/// An RPC response for the latest aggregated proof info.
#[derive(Debug, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProofInfoResponse {
    /// Initial rollup height
    pub initial_slot_number: u64,
    /// Final rollup height.
    pub final_slot_number: u64,
}

/// An RPC response for the latest aggregated proof.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct AggregatedProofResponse {
    /// Aggregated proof.
    pub proof: SerializedAggregatedProof,
}

/// An RPC response for the module specific event
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct PaginatedEventResponse<E> {
    /// A value representing the module event serialized as json
    pub events_response: Vec<E>,
    /// Module name that the event belongs to
    pub next: Option<String>,
}

/// A [`LedgerStateProvider`] provides a way to query the ledger for information about
/// slots, batches, transactions, and events.
#[async_trait]
pub trait LedgerStateProvider {
    /// The error type for fallible methods on this trait.
    type Error: ToString + Send + Sync + 'static;

    /// Get the latest rollup height in the ledger.
    async fn get_head_slot_number(&self) -> Result<SlotNumber, Self::Error>;

    /// Get the latest rollup height in the ledger.
    async fn get_latest_finalized_slot_number(&self) -> Result<SlotNumber, Self::Error>;

    /// Get the latest slot in the ledger.
    async fn get_head<B, T, E>(
        &self,
        query_mode: QueryMode,
    ) -> Result<SlotResponse<B, T, E>, Self::Error>
    where
        B: DeserializeOwned + Clone + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        let head_number = self.get_head_slot_number().await?;
        self.get_slot_by_number(head_number, query_mode)
            .await
            .map(|opt| opt.expect("Head slot should always exist. This is a bug, please report it"))
    }

    /// Get a list of slots by id. The IDs need not be ordered.
    async fn get_slots<B, T, E>(
        &self,
        slot_ids: &[SlotIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<SlotResponse<B, T, E>>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync;

    /// Get a list of batches by id. The IDs need not be ordered.
    async fn get_batches<B, T, E>(
        &self,
        batch_ids: &[BatchIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<BatchResponse<B, T, E>>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync;

    /// Get a list of transactions by id. The IDs need not be ordered.
    async fn get_transactions<T, E>(
        &self,
        tx_ids: &[TxIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<TxResponse<T, E>>>, Self::Error>
    where
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync;

    /// Get events by id. The IDs need not be ordered.
    async fn get_events<E>(
        &self,
        event_ids: &[EventIdentifier],
    ) -> Result<Vec<Option<E>>, Self::Error>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync;

    /// Get all events from a slot with an optional prefix filter. If a filter
    /// is not provided, all events from that slot are returned.
    async fn get_filtered_slot_events<B, T, E>(
        &self,
        slot_id: &SlotIdentifier,
        event_key_prefix_filter: Option<Vec<u8>>,
    ) -> Result<Vec<E>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error>
            + Send
            + Sync
            + DeserializeOwned;

    /// Get a single slot by hash.
    async fn get_slot_by_hash<B, T, E>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<SlotResponse<B, T, E>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_slots(&[SlotIdentifier::Hash(*hash)], query_mode)
            .await
            .map(|mut batches: Vec<Option<SlotResponse<B, T, E>>>| batches.pop().unwrap_or(None))
    }

    /// Get a single batch by hash.
    async fn get_batch_by_hash<B, T, E>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<BatchResponse<B, T, E>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_batches(&[BatchIdentifier::Hash(*hash)], query_mode)
            .await
            .map(|mut batches: Vec<Option<BatchResponse<B, T, E>>>| batches.pop().unwrap_or(None))
    }

    /// Get a single transaction by hash.
    async fn get_tx_by_hash<T, E>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<(u64, TxResponse<T, E>)>, Self::Error>
    where
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        let tx_id = self
            .resolve_tx_identifier(&TxIdentifier::Hash(*hash))
            .await?;
        if let Some(tx_id) = tx_id {
            let mut txs = self
                .get_transactions(&[TxIdentifier::Number(tx_id)], query_mode)
                .await?;
            assert_eq!(txs.len(), 1, "Tx id was in storage, but a query for it returned {} results. This is a bug, please report it.", txs.len());
            let tx = txs.pop().flatten().unwrap(); // Safety: We just asserted that there is exactly one result
            Ok(Some((tx_id, tx)))
        } else {
            Ok(None)
        }
    }

    /// Get a list of transaction numbers by hash. Since a tx hash itself
    /// may not be unique, this returns a list of unique tx numbers associated
    /// with that hash, which numbers may then be used to query the transaction.
    async fn get_tx_numbers_by_hash(&self, hash: &[u8; 32]) -> Result<Vec<u64>, Self::Error>;

    /// Get a single slot by number.
    async fn get_slot_by_number<B, T, E>(
        &self,
        number: SlotNumber,
        query_mode: QueryMode,
    ) -> Result<Option<SlotResponse<B, T, E>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_slots(&[SlotIdentifier::Number(number)], query_mode)
            .await
            .map(|mut slots| slots.pop().unwrap_or(None))
    }

    /// Get a single batch by number.
    async fn get_batch_by_number<B, T, E>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<BatchResponse<B, T, E>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_batches(&[BatchIdentifier::Number(number)], query_mode)
            .await
            .map(|mut batches| batches.pop().unwrap_or(None))
    }

    /// Get a single event by number.
    async fn get_event_by_number<E>(&self, number: u64) -> Result<Option<E>, Self::Error>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_events::<E>(&[EventIdentifier::Number(number)])
            .await
            .map(|mut events| events.pop().flatten())
    }

    /// Gets the latest (as it, by event ID) event, if any exists.
    async fn get_latest_event_number(&self) -> Result<Option<u64>, Self::Error>;

    /// Get events by transaction hash.
    async fn get_events_by_txn_hash<E>(&self, txn_hash: &[u8; 32]) -> Result<Vec<E>, Self::Error>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync;

    /// Get events by transaction number.
    async fn get_events_by_txn_number<E>(&self, txn_num: u64) -> Result<Vec<E>, Self::Error>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync;

    /// Get a single tx by number.
    async fn get_tx_by_number<T, E>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<TxResponse<T, E>>, Self::Error>
    where
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_transactions(&[TxIdentifier::Number(number)], query_mode)
            .await
            .map(|mut txs| txs.pop().unwrap_or(None))
    }

    /// Get a range of slots. This query is the most efficient way to
    /// fetch large numbers of slots, since it allows for easy batching of
    /// db queries for adjacent items.
    async fn get_slots_range<B, T, E>(
        &self,
        start: SlotNumber,
        end: SlotNumber,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<SlotResponse<B, T, E>>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync;

    /// Get a range of batches. This query is the most efficient way to
    /// fetch large numbers of batches, since it allows for easy batching of
    /// db queries for adjacent items.
    async fn get_batches_range<B, T, E>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<BatchResponse<B, T, E>>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync;

    /// Get a range of batches. This query is the most efficient way to
    /// fetch large numbers of transactions, since it allows for easy batching of
    /// db queries for adjacent items.
    async fn get_transactions_range<T, E>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<TxResponse<T, E>>>, Self::Error>
    where
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync;

    /// Resolve a [`SlotIdentifier`] into a rollup height.
    async fn resolve_slot_identifier(
        &self,
        slot_id: &SlotIdentifier,
    ) -> Result<Option<SlotNumber>, Self::Error>;

    /// Resolve a [`BatchIdentifier`] into a batch number.
    async fn resolve_batch_identifier(
        &self,
        batch_id: &BatchIdentifier,
    ) -> Result<Option<u64>, Self::Error>;

    /// Resolve a [`TxIdentifier`] into a transaction number.
    async fn resolve_tx_identifier(&self, tx_id: &TxIdentifier)
        -> Result<Option<u64>, Self::Error>;

    /// Resolve a [`EventIdentifier`] into an event number.
    async fn resolve_event_identifier(
        &self,
        event_id: &EventIdentifier,
    ) -> Result<Option<u64>, Self::Error>;

    /// Get the most recent aggregated proof, if any.
    async fn get_latest_aggregated_proof(&self) -> anyhow::Result<Option<AggregatedProofResponse>>;

    /// Get a notification each time a slot is processed
    fn subscribe_slots(&self) -> BoxStream<'static, SlotNumber>;

    /// Get a notification each time a slot is finalized
    fn subscribe_finalized_slots(&self) -> BoxStream<'static, SlotNumber>;

    /// Get a notification each time an aggregated proof is processed
    // https://github.com/Sovereign-Labs/sovereign-sdk/issues/1161
    fn subscribe_proof_saved(&self) -> BoxStream<'static, AggregatedProofResponse>;
}
