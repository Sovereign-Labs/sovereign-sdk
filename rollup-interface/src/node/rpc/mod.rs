use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::stf::Event;

/// An identifier that specifies a single batch
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum BatchIdentifier {
    Hash([u8; 32]),
    SlotIdAndIndex((SlotIdentifier, u64)),
    /// The monotonically increasing number of the batch, ordered by the DA layer For example, if the genesis slot
    /// contains 0 batches, slot 1 contains 2 txs, and slot 3 contains 3 txs,
    /// the last batch in block 3 would have number 5. The counter never resets.
    Number(u64),
}

/// An identifier that specifies a single transaction
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum TxIdentifier {
    Hash([u8; 32]),
    BatchIdAndIndex((BatchIdentifier, u64)),
    /// The monotonically increasing number of the tx, ordered by the DA layer For example, if genesis
    /// contains 0 txs, batch 1 contains 8 txs, and batch 3 contains 7 txs,
    /// the last tx in batch 3 would have number 15. The counter never resets.
    Number(u64),
}

/// An identifier that specifies a single event
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum EventIdentifier {
    TxIdAndIndex((TxIdentifier, u64)),
    TxIdAndKey((TxIdentifier, Vec<u8>)),
    /// The monotonically increasing number of the event, ordered by the DA layer For example, if the first tx
    /// contains 7 events, tx 2 contains 11 events, and tx 3 contains 7 txs,
    /// the last event in tx 3 would have number 25. The counter never resets.
    Number(u64),
}

/// An identifier for a group of related events
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum EventGroupIdentifier {
    TxId(TxIdentifier),
    Key(Vec<u8>),
}

/// An identifier that specifies a single slot
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum SlotIdentifier {
    Hash([u8; 32]), // the hash of a da block
    Number(u64),    // the block number of a da block
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryMode {
    /// Returns the minimal parent struct with no minimal about its children.
    /// For example, a compact "slot" response would contain a range of
    Compact,
    Standard,
    Full,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct SlotResponse<B, Tx> {
    pub number: u64,
    pub hash: [u8; 32],
    pub batch_range: std::ops::Range<u64>,
    pub batches: Option<Vec<ItemOrHash<BatchResponse<B, Tx>>>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BatchResponse<B, Tx> {
    pub hash: [u8; 32],
    pub tx_range: std::ops::Range<u64>,
    pub txs: Option<Vec<ItemOrHash<TxResponse<Tx>>>>,
    pub custom_receipt: B,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct TxResponse<Tx> {
    pub hash: [u8; 32],
    pub event_range: std::ops::Range<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Vec<u8>>,
    #[serde(flatten)]
    pub custom_receipt: Tx,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ItemOrHash<T> {
    Hash([u8; 32]),
    Full(T),
}

pub trait LedgerRpcProvider {
    fn get_head<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
    ) -> Result<Option<SlotResponse<B, T>>, anyhow::Error>;

    fn get_slots<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        slot_ids: &[SlotIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<SlotResponse<B, T>>>, anyhow::Error>;
    fn get_batches<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        batch_ids: &[BatchIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<BatchResponse<B, T>>>, anyhow::Error>;
    fn get_transactions<T: DeserializeOwned>(
        &self,
        tx_ids: &[TxIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<TxResponse<T>>>, anyhow::Error>;
    fn get_events(
        &self,
        event_ids: &[EventIdentifier],
    ) -> Result<Vec<Option<Event>>, anyhow::Error>;
    fn get_slot_by_hash<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<SlotResponse<B, T>>, anyhow::Error>;
    fn get_batch_by_hash<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<BatchResponse<B, T>>, anyhow::Error>;
    fn get_tx_by_hash<T: DeserializeOwned>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<TxResponse<T>>, anyhow::Error>;
    fn get_slot_by_number<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<SlotResponse<B, T>>, anyhow::Error>;
    fn get_batch_by_number<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<BatchResponse<B, T>>, anyhow::Error>;
    fn get_event_by_number(&self, number: u64) -> Result<Option<Event>, anyhow::Error>;
    fn get_tx_by_number<T: DeserializeOwned>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<TxResponse<T>>, anyhow::Error>;
    fn get_slots_range<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<SlotResponse<B, T>>>, anyhow::Error>;
    fn get_batches_range<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<BatchResponse<B, T>>>, anyhow::Error>;
    fn get_transactions_range<T: DeserializeOwned>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<TxResponse<T>>>, anyhow::Error>;
}
