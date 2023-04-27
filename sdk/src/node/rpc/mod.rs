use serde::{Deserialize, Serialize};

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

pub trait LedgerRpcProvider {
    type SlotResponse: Serialize;
    type BatchResponse: Serialize;
    type TxResponse: Serialize;
    type EventResponse: Serialize;

    fn get_head(&self) -> Result<Option<Self::SlotResponse>, anyhow::Error>;

    fn get_slots(
        &self,
        slot_ids: &[SlotIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::SlotResponse>>, anyhow::Error>;
    fn get_batches(
        &self,
        batch_ids: &[BatchIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::BatchResponse>>, anyhow::Error>;
    fn get_transactions(
        &self,
        tx_ids: &[TxIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::TxResponse>>, anyhow::Error>;
    fn get_events(
        &self,
        event_ids: &[EventIdentifier],
    ) -> Result<Vec<Option<Self::EventResponse>>, anyhow::Error>;
    fn get_slot_by_hash(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<Self::SlotResponse>, anyhow::Error>;
    fn get_batch_by_hash(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<Self::BatchResponse>, anyhow::Error>;
    fn get_tx_by_hash(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<Self::TxResponse>, anyhow::Error>;
    fn get_slot_by_number(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<Self::SlotResponse>, anyhow::Error>;
    fn get_batch_by_number(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<Self::BatchResponse>, anyhow::Error>;
    fn get_event_by_number(
        &self,
        number: u64,
    ) -> Result<Option<Self::EventResponse>, anyhow::Error>;
    fn get_tx_by_number(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<Self::TxResponse>, anyhow::Error>;
    fn get_slots_range(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::SlotResponse>>, anyhow::Error>;
    fn get_batches_range(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::BatchResponse>>, anyhow::Error>;
    fn get_transactions_range(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::TxResponse>>, anyhow::Error>;
}
