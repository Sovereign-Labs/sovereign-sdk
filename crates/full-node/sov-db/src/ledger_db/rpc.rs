use anyhow::{bail, Context};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use rockbound::cache::delta_reader::DeltaReader;
use rockbound::{Schema, SeekKeyEncoder};
use serde::de::DeserializeOwned;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::node::ledger_api::{
    AggregatedProofResponse, BatchIdAndOffset, BatchIdentifier, BatchResponse, EventIdentifier,
    FinalityStatus, ItemOrHash, LedgerStateProvider, QueryMode, SlotIdAndOffset, SlotIdentifier,
    SlotResponse, TxIdAndOffset, TxIdentifier, TxResponse,
};
use sov_rollup_interface::stf::{StoredEvent, TxReceiptContents};
use tokio_stream::wrappers::{BroadcastStream, WatchStream};
use tracing::error;

use crate::ledger_db::rpc_constants::{
    MAX_BATCHES_PER_REQUEST, MAX_EVENTS_PER_REQUEST, MAX_SLOTS_PER_REQUEST,
    MAX_TRANSACTIONS_PER_REQUEST,
};
use crate::ledger_db::{LedgerDb, DB_LOCK_POISONED};
use crate::schema::tables::{
    BatchByHash, BatchByNumber, EventByNumber, FinalizedSlots, ProofByUniqueId, SlotByHash,
    SlotByNumber, TxByHash, TxByNumber,
};
use crate::schema::types::{
    BatchNumber, EventNumber, LatestFinalizedSlotSingleton, StoredBatch, StoredSlot,
    StoredTransaction, TxNumber,
};

/// Wrapper around cloned [`DeltaReader`].
/// So all reads are consistent inside a call.
pub(crate) struct LedgerRpcReader {
    pub(crate) db: DeltaReader,
}

impl LedgerRpcReader {
    async fn get_head_slot_number(&self) -> anyhow::Result<Option<SlotNumber>> {
        self.db
            .get_largest_async::<SlotByNumber>()
            .await
            .map(|opt| opt.map(|t| t.0))
    }

    async fn get_latest_finalized_slot_number(&self) -> anyhow::Result<SlotNumber> {
        let finalized_slot_num = self
            .db
            .get_async::<FinalizedSlots>(&LatestFinalizedSlotSingleton)
            .await?;
        Ok(finalized_slot_num.unwrap_or_default())
    }

    async fn get_slots<B, T, E>(
        &self,
        slot_ids: &[SlotIdentifier],
        query_mode: QueryMode,
    ) -> anyhow::Result<Vec<Option<SlotResponse<B, T, E>>>>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        anyhow::ensure!(
            slot_ids.len() <= MAX_SLOTS_PER_REQUEST as usize,
            "requested too many slots. Requested: {}. Max: {}",
            slot_ids.len(),
            MAX_SLOTS_PER_REQUEST
        );
        // TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/191 Sort the input
        //      and use an iterator instead of querying for each slot individually
        let mut out = Vec::with_capacity(slot_ids.len());
        for slot_id in slot_ids {
            let slot_num = self.resolve_slot_identifier(slot_id).await?;
            out.push(match slot_num {
                Some(num) => {
                    if let Some(stored_slot) = self.db.get_async::<SlotByNumber>(&num).await? {
                        Some(
                            self.populate_slot_response(num, stored_slot, query_mode)
                                .await?,
                        )
                    } else {
                        None
                    }
                }
                None => None,
            });
        }
        Ok(out)
    }

    async fn get_batches<B, T, E>(
        &self,
        batch_ids: &[BatchIdentifier],
        query_mode: QueryMode,
    ) -> anyhow::Result<Vec<Option<BatchResponse<B, T, E>>>>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        // TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/191 Sort the input
        //      and use an iterator instead of querying for each slot individually
        let mut out = Vec::with_capacity(batch_ids.len());
        for batch_id in batch_ids {
            let batch_num = self.resolve_batch_identifier(batch_id).await?;
            out.push(match batch_num {
                Some(num) => {
                    if let Some(stored_batch) = self
                        .db
                        .get_async::<BatchByNumber>(&BatchNumber(num))
                        .await?
                    {
                        Some(
                            self.populate_batch_response(stored_batch, query_mode)
                                .await?,
                        )
                    } else {
                        None
                    }
                }
                None => None,
            });
        }
        Ok(out)
    }

    async fn get_transactions<T, E>(
        &self,
        tx_ids: &[TxIdentifier],
        query_mode: QueryMode,
    ) -> anyhow::Result<Vec<Option<TxResponse<T, E>>>>
    where
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        // TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/191 Sort the input
        //      and use an iterator instead of querying for each slot individually
        let mut out: Vec<Option<TxResponse<T, E>>> = Vec::with_capacity(tx_ids.len());
        for id in tx_ids {
            let num = self.resolve_tx_identifier(id).await?;
            out.push(match num {
                Some(num) => {
                    if let Some(tx) = self.db.get_async::<TxByNumber>(&TxNumber(num)).await? {
                        Some(Self::populate_tx_response(self, tx, query_mode).await?)
                    } else {
                        None
                    }
                }
                None => None,
            });
        }
        Ok(out)
    }

    async fn get_tx_numbers_by_hash(&self, hash: &[u8; 32]) -> anyhow::Result<Vec<u64>> {
        let tx_range = (*hash, TxNumber(0))..(*hash, TxNumber(u64::MAX));
        self.db
            .collect_in_range_async::<TxByHash, ([u8; 32], TxNumber)>(tx_range)
            .await
            .map(|v| {
                v.iter()
                    .map(|((_, tx_num), _)| tx_num.0)
                    .collect::<Vec<_>>()
            })
    }

    pub(crate) async fn get_events<E>(
        &self,
        event_ids: &[EventIdentifier],
    ) -> anyhow::Result<Vec<Option<E>>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        // TODO: Sort the input and use an iterator instead of querying for each slot individually
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/191
        let mut out = Vec::with_capacity(event_ids.len());
        for id in event_ids {
            let num = self.resolve_event_identifier(id).await?;
            out.push(
                match num {
                    Some(num) => self
                        .db
                        .get_async::<EventByNumber>(&EventNumber(num))
                        .await?
                        .map(|serialized_event| (num, &serialized_event).try_into()),
                    None => None,
                }
                .transpose()?,
            );
        }
        Ok(out)
    }

    pub(crate) async fn get_latest_event_number(&self) -> anyhow::Result<Option<u64>> {
        match self.db.get_largest_async::<EventByNumber>().await? {
            Some(e) => Ok(Some(e.0 .0)),
            None => Ok(None),
        }
    }

    async fn collect_transaction_numbers(
        &self,
        tx_range: std::ops::Range<([u8; 32], TxNumber)>,
    ) -> anyhow::Result<Vec<TxNumber>> {
        Ok(self
            .db
            .collect_in_range_async::<TxByHash, ([u8; 32], TxNumber)>(tx_range)
            .await?
            .into_iter()
            .map(|((_, tx_num), _)| tx_num)
            .collect())
    }

    async fn get_events_by_txn_hash<E>(&self, tx_hash: &[u8; 32]) -> anyhow::Result<Vec<E>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        let tx_range = (*tx_hash, TxNumber(0))..(*tx_hash, TxNumber(u64::MAX));
        let tx_numbers = self
            .collect_transaction_numbers(tx_range)
            .await
            .with_context(|| {
                format!(
                    "Failed to query transaction with hash: 0x{}",
                    hex::encode(tx_hash)
                )
            })?;

        if tx_numbers.is_empty() {
            bail!(
                "Transaction with hash: 0x{} does not exist in storage",
                hex::encode(tx_hash)
            )
        }

        let mut events_response = vec![];
        for tx_num in tx_numbers {
            // TODO: Atomicity
            let events = self
                .get_events_by_txn_number::<E>(tx_num.0)
                .await
                .with_context(|| {
                    format!("Resolved transaction hash {} to tx number {}, but failed to resolve find the events for that number", hex::encode(tx_hash), tx_num.0)
                })?;
            events_response.extend(events.into_iter());
        }
        Ok(events_response)
    }

    async fn get_events_by_txn_number<E>(&self, txn_num: u64) -> anyhow::Result<Vec<E>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        let stored_txn = self
            .db
            .get_async::<TxByNumber>(&TxNumber(txn_num))
            .await
            .with_context(|| {
                format!("Failed to query transaction with number: {txn_num} from storage")
            })?
            .with_context(|| {
                format!("Transaction with number: {txn_num} does not exist in storage")
            })?;
        // Can't map over stored_txn.events because no Step trait, so doing this manually
        // TODO: can we implement the Step trait
        // let event_ids: Vec<EventIdentifier> =
        //     stored_txn.events.map(EventIdentifier::Number).collect();

        let mut event_ids = Vec::new();
        let EventNumber(start) = stored_txn.events.start;
        let EventNumber(end) = stored_txn.events.end;
        for number in start..end {
            event_ids.push(EventIdentifier::Number(number));
        }

        let events_response: Vec<E> = self
            .get_events::<E>(&event_ids)
            .await?
            .into_iter()
            .flatten()
            .collect();
        Ok(events_response)
    }

    /// Gets all batches with numbers `range.start` to `range.end`. If `range.end` is outside
    /// the range of the database, the result will be smaller than the requested range.
    /// Note that this method blindly preallocates for the requested range, so it should not be exposed
    /// directly via rpc.
    pub(crate) async fn get_batch_range(
        &self,
        range: &std::ops::Range<BatchNumber>,
    ) -> anyhow::Result<Vec<StoredBatch>> {
        self.get_data_range::<BatchByNumber, _, _>(range).await
    }

    /// Gets all transactions with numbers `range.start` to `range.end`. If `range.end` is outside
    /// the range of the database, the result will be smaller than the requested range.
    /// Note that this method blindly preallocates for the requested range, so it should not be exposed
    /// directly via rpc.
    pub(crate) async fn get_tx_range(
        &self,
        range: &std::ops::Range<TxNumber>,
    ) -> anyhow::Result<Vec<StoredTransaction>> {
        self.get_data_range::<TxByNumber, _, _>(range).await
    }

    /// Gets all events with numbers `range.start` to `range.end`. If `range.end` is outside
    /// the range of the database, the result will be smaller than the requested range.
    /// Note that this method blindly preallocates for the requested range, so it should not be exposed
    /// directly via rpc.
    pub(crate) async fn get_event_range(
        &self,
        range: &std::ops::Range<EventNumber>,
    ) -> anyhow::Result<Vec<StoredEvent>> {
        self.get_data_range::<EventByNumber, _, _>(range).await
    }

    pub(crate) async fn get_data_range<T, K, V>(
        &self,
        range: &std::ops::Range<K>,
    ) -> anyhow::Result<Vec<V>>
    where
        T: Schema<Key = K, Value = V>,
        K: Into<u64> + Copy + SeekKeyEncoder<T>,
    {
        let raw_out = self.db.collect_in_range_async(range.clone()).await?;
        let mut out = Vec::with_capacity(raw_out.len());
        for (_, value) in raw_out {
            out.push(value);
        }
        Ok(out)
    }

    async fn resolve_slot_identifier(
        &self,
        slot_id: &SlotIdentifier,
    ) -> anyhow::Result<Option<SlotNumber>> {
        match slot_id {
            SlotIdentifier::Hash(hash) => self.db.get_async::<SlotByHash>(hash).await,
            SlotIdentifier::Number(num) => Ok(Some(*num)),
        }
    }

    async fn resolve_batch_identifier(
        &self,
        batch_id: &BatchIdentifier,
    ) -> anyhow::Result<Option<u64>> {
        match batch_id {
            BatchIdentifier::Hash(hash) => self
                .db
                .get_async::<BatchByHash>(hash)
                .await
                .map(|id_opt| id_opt.map(|id| id.0)),
            BatchIdentifier::Number(num) => Ok(Some(*num)),
            BatchIdentifier::SlotIdAndOffset(SlotIdAndOffset { slot_id, offset }) => {
                if let Some(slot_num) = self.resolve_slot_identifier(slot_id).await? {
                    Ok(self
                        .db
                        .get_async::<SlotByNumber>(&slot_num)
                        .await?
                        .map(|slot: StoredSlot| slot.batches.start.0 + offset))
                } else {
                    Ok(None)
                }
            }
        }
    }

    async fn resolve_tx_identifier(&self, tx_id: &TxIdentifier) -> anyhow::Result<Option<u64>> {
        match tx_id {
            TxIdentifier::Hash(hash) => {
                // When someone queries for a single TX by hash, we assume they want the first one.
                // This heuristic is better than our old one (implicitly returning the latest instance), because
                // it's more likely that a transaction gets succeeds on its first inclusion than on a second one.
                // (This is because transactions with *future* nonces rarely get included, but transactions with
                // past nonces can get included easily by racing sequencers.)
                let tx_range = (*hash, TxNumber(0))..(*hash, TxNumber(u64::MAX));
                let tx_numbers = self
                    .collect_transaction_numbers(tx_range)
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to query transaction with hash: 0x{}",
                            hex::encode(hash)
                        )
                    })?;
                Ok(tx_numbers.first().map(|tx_num| tx_num.0))
            }
            TxIdentifier::Number(num) => Ok(Some(*num)),
            TxIdentifier::BatchIdAndOffset(BatchIdAndOffset { batch_id, offset }) => {
                if let Some(batch_num) = self.resolve_batch_identifier(batch_id).await? {
                    Ok(self
                        .db
                        .get_async::<BatchByNumber>(&BatchNumber(batch_num))
                        .await?
                        .map(|batch: StoredBatch| batch.txs.start.0 + offset))
                } else {
                    Ok(None)
                }
            }
        }
    }

    async fn resolve_event_identifier(
        &self,
        event_id: &EventIdentifier,
    ) -> anyhow::Result<Option<u64>> {
        match event_id {
            EventIdentifier::TxIdAndOffset(TxIdAndOffset { tx_id, offset }) => {
                if let Some(tx_num) = self.resolve_tx_identifier(tx_id).await? {
                    Ok(self
                        .db
                        .get_async::<TxByNumber>(&TxNumber(tx_num))
                        .await?
                        .map(|tx| tx.events.start.0 + offset))
                } else {
                    Ok(None)
                }
            }
            EventIdentifier::Number(num) => Ok(Some(*num)),
        }
    }

    async fn populate_tx_response<T: TxReceiptContents, E>(
        &self,
        tx: StoredTransaction,
        mode: QueryMode,
    ) -> anyhow::Result<TxResponse<T, E>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        Ok(match mode {
            QueryMode::Compact => tx.try_into()?,

            QueryMode::Standard | QueryMode::Full => {
                let events = self.get_event_range(&tx.events).await?;
                let events = (tx.events.start.0..=tx.events.end.0)
                    .zip(events.into_iter())
                    .map(|(event_num, event)| (event_num, &event).try_into())
                    .collect::<Result<Vec<E>, _>>()?;
                let mut tx_response: TxResponse<T, E> = tx.try_into()?;
                tx_response.events = Some(events);
                tx_response
            }
        })
    }

    async fn populate_batch_response<B: DeserializeOwned, T: TxReceiptContents, E>(
        &self,
        batch: StoredBatch,
        mode: QueryMode,
    ) -> anyhow::Result<BatchResponse<B, T, E>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        Ok(match mode {
            QueryMode::Compact => batch.to_batch_response()?,

            QueryMode::Standard => {
                let txs = self.get_tx_range(&batch.txs).await?;
                let tx_hashes = Some(
                    txs.into_iter()
                        .map(|tx| ItemOrHash::Hash(tx.hash))
                        .collect(),
                );

                let mut batch_response: BatchResponse<B, T, E> = batch.to_batch_response()?;
                batch_response.txs = tx_hashes;
                batch_response
            }
            QueryMode::Full => {
                let num_txs = (batch.txs.end.0 - batch.txs.start.0) as usize;
                let mut txs = Vec::with_capacity(num_txs);
                for tx in self.get_tx_range(&batch.txs).await? {
                    let tx = self.populate_tx_response(tx, mode).await?;
                    txs.push(ItemOrHash::Full(tx));
                }

                let mut batch_response: BatchResponse<B, T, E> = batch.to_batch_response()?;
                batch_response.txs = Some(txs);
                batch_response
            }
        })
    }

    async fn populate_slot_response<B: DeserializeOwned, T: TxReceiptContents, E>(
        &self,
        number: SlotNumber,
        slot: StoredSlot,
        mode: QueryMode,
    ) -> anyhow::Result<SlotResponse<B, T, E>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        let state_root = slot.state_root.as_ref().to_vec();
        let finality_status = if self.get_latest_finalized_slot_number().await? >= number {
            FinalityStatus::Finalized
        } else {
            FinalityStatus::Pending
        };

        Ok(match mode {
            QueryMode::Compact => SlotResponse {
                number: number.get(),
                hash: slot.hash,
                state_root,
                batch_range: slot.batches.start.into()..slot.batches.end.into(),
                batches: None,
                finality_status,
                timestamp: slot.timestamp,
            },
            QueryMode::Standard => {
                let batches = self.get_batch_range(&slot.batches).await?;
                let batch_hashes = Some(
                    batches
                        .into_iter()
                        .map(|batch| ItemOrHash::Hash(batch.hash))
                        .collect(),
                );
                SlotResponse {
                    number: number.get(),
                    hash: slot.hash,
                    state_root,
                    batch_range: slot.batches.start.into()..slot.batches.end.into(),
                    batches: batch_hashes,
                    finality_status,
                    timestamp: slot.timestamp,
                }
            }
            QueryMode::Full => {
                let num_batches = (slot.batches.end.0 - slot.batches.start.0) as usize;
                let mut batches = Vec::with_capacity(num_batches);
                for batch in self.get_batch_range(&slot.batches).await? {
                    batches.push(ItemOrHash::Full(
                        self.populate_batch_response(batch, mode).await?,
                    ));
                }

                SlotResponse {
                    number: number.get(),
                    hash: slot.hash,
                    state_root,
                    batch_range: slot.batches.start.into()..slot.batches.end.into(),
                    batches: Some(batches),
                    finality_status,
                    timestamp: slot.timestamp,
                }
            }
        })
    }
}

#[async_trait]
impl LedgerStateProvider for LedgerDb {
    type Error = anyhow::Error;

    async fn get_head_slot_number(&self) -> Result<SlotNumber, Self::Error> {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader
            .get_head_slot_number()
            .await
            .map(Option::unwrap_or_default)
    }

    async fn get_latest_finalized_slot_number(&self) -> Result<SlotNumber, Self::Error> {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.get_latest_finalized_slot_number().await
    }

    async fn get_slots<B, T, E>(
        &self,
        slot_ids: &[SlotIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<SlotResponse<B, T, E>>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.get_slots(slot_ids, query_mode).await
    }

    async fn get_batches<B, T, E>(
        &self,
        batch_ids: &[BatchIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<BatchResponse<B, T, E>>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        anyhow::ensure!(
            batch_ids.len() <= MAX_BATCHES_PER_REQUEST as usize,
            "requested too many batches. Requested: {}. Max: {}",
            batch_ids.len(),
            MAX_BATCHES_PER_REQUEST
        );
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.get_batches(batch_ids, query_mode).await
    }

    async fn get_transactions<T, E>(
        &self,
        tx_ids: &[TxIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<TxResponse<T, E>>>, Self::Error>
    where
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        anyhow::ensure!(
            tx_ids.len() <= MAX_TRANSACTIONS_PER_REQUEST as usize,
            "requested too many transactions. Requested: {}. Max: {}",
            tx_ids.len(),
            MAX_TRANSACTIONS_PER_REQUEST
        );
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.get_transactions(tx_ids, query_mode).await
    }

    async fn get_events<E>(
        &self,
        event_ids: &[EventIdentifier],
    ) -> Result<Vec<Option<E>>, Self::Error>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        anyhow::ensure!(
            event_ids.len() <= MAX_EVENTS_PER_REQUEST as usize,
            "requested too many events. Requested: {}. Max: {}",
            event_ids.len(),
            MAX_EVENTS_PER_REQUEST
        );
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.get_events(event_ids).await
    }

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
            + DeserializeOwned,
    {
        let slot_not_found_err = || anyhow::anyhow!("Slot `{:?}` not found", slot_id);

        let slot_num = self
            .resolve_slot_identifier(slot_id)
            .await?
            .ok_or_else(slot_not_found_err)?;
        let slot: SlotResponse<B, T, E> = self
            .get_slot_by_number(slot_num, QueryMode::Full)
            .await?
            .ok_or_else(slot_not_found_err)?;

        let batches = slot
            .batches
            .unwrap_or_default()
            .into_iter()
            .filter_map(|b| match b {
                ItemOrHash::Full(b) => Some(b),
                _ => None,
            });
        let txs = batches.flat_map(|b| {
            b.txs
                .unwrap_or_default()
                .into_iter()
                .filter_map(|t| match t {
                    ItemOrHash::Full(t) => Some(t),
                    _ => None,
                })
        });
        let event_nums = txs.flat_map(|t| t.event_range);

        let mut events = vec![];

        let db = self.db.read().expect(DB_LOCK_POISONED).clone();
        for event_num in event_nums {
            let event = db
                .get_async::<EventByNumber>(&EventNumber(event_num))
                .await?
                .ok_or_else(|| anyhow::anyhow!("Event not found but should be present"))?;

            if let Some(prefix) = &event_key_prefix_filter {
                if !event.key().inner().starts_with(prefix) {
                    continue;
                }
            }

            events.push((event_num, &event).try_into()?);
        }

        Ok(events)
    }

    // Get X by hash
    async fn get_slot_by_hash<B, T, E>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> anyhow::Result<Option<SlotResponse<B, T, E>>>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_slots(&[SlotIdentifier::Hash(*hash)], query_mode)
            .await
            .map(|mut batches: Vec<Option<SlotResponse<B, T, E>>>| batches.pop().unwrap_or(None))
    }

    async fn get_batch_by_hash<B, T, E>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> anyhow::Result<Option<BatchResponse<B, T, E>>>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_batches(&[BatchIdentifier::Hash(*hash)], query_mode)
            .await
            .map(|mut batches: Vec<Option<BatchResponse<B, T, E>>>| batches.pop().unwrap_or(None))
    }

    async fn get_tx_numbers_by_hash(&self, hash: &[u8; 32]) -> Result<Vec<u64>, Self::Error> {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.get_tx_numbers_by_hash(hash).await
    }

    // Get X by number
    async fn get_slot_by_number<B, T, E>(
        &self,
        number: SlotNumber,
        query_mode: QueryMode,
    ) -> anyhow::Result<Option<SlotResponse<B, T, E>>>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_slots(&[SlotIdentifier::Number(number)], query_mode)
            .await
            .map(|mut slots: Vec<Option<SlotResponse<B, T, E>>>| slots.pop().unwrap_or(None))
    }

    async fn get_batch_by_number<B, T, E>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> anyhow::Result<Option<BatchResponse<B, T, E>>>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_batches(&[BatchIdentifier::Number(number)], query_mode)
            .await
            .map(|mut slots| slots.pop().unwrap_or(None))
    }

    async fn get_event_by_number<E>(&self, number: u64) -> anyhow::Result<Option<E>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        self.get_events::<E>(&[EventIdentifier::Number(number)])
            .await
            .map(|mut events| events.pop().flatten())
    }

    async fn get_latest_event_number(&self) -> Result<Option<u64>, Self::Error> {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.get_latest_event_number().await
    }

    async fn get_events_by_txn_hash<E>(&self, txn_hash: &[u8; 32]) -> anyhow::Result<Vec<E>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.get_events_by_txn_hash(txn_hash).await
    }

    async fn get_events_by_txn_number<E>(&self, txn_num: u64) -> anyhow::Result<Vec<E>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.get_events_by_txn_number(txn_num).await
    }

    async fn get_slots_range<B, T, E>(
        &self,
        start: SlotNumber,
        end: SlotNumber,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<SlotResponse<B, T, E>>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        anyhow::ensure!(start <= end, "start must be <= end");
        anyhow::ensure!(
            end.delta(start) <= MAX_SLOTS_PER_REQUEST,
            "requested slot range too large. Max: {}",
            MAX_SLOTS_PER_REQUEST
        );
        let ids: Vec<_> = (start.get()..=end.get())
            .map(|x| SlotIdentifier::Number(SlotNumber::new_dangerous(x)))
            .collect();
        self.get_slots(&ids, query_mode).await
    }

    async fn get_batches_range<B, T, E>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<BatchResponse<B, T, E>>>, Self::Error>
    where
        B: DeserializeOwned + Send + Sync,
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        anyhow::ensure!(start <= end, "start must be <= end");
        anyhow::ensure!(
            end - start <= MAX_BATCHES_PER_REQUEST,
            "requested batch range too large. Max: {}",
            MAX_BATCHES_PER_REQUEST
        );
        let ids: Vec<_> = (start..=end).map(BatchIdentifier::Number).collect();
        self.get_batches(&ids, query_mode).await
    }

    async fn get_transactions_range<T, E>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<TxResponse<T, E>>>, Self::Error>
    where
        T: TxReceiptContents,
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        anyhow::ensure!(start <= end, "start must be <= end");
        anyhow::ensure!(
            end - start <= MAX_TRANSACTIONS_PER_REQUEST,
            "requested transaction range too large. Max: {}",
            MAX_TRANSACTIONS_PER_REQUEST
        );
        let ids: Vec<_> = (start..=end).map(TxIdentifier::Number).collect();
        self.get_transactions(&ids, query_mode).await
    }

    async fn resolve_slot_identifier(
        &self,
        slot_id: &SlotIdentifier,
    ) -> Result<Option<SlotNumber>, Self::Error> {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.resolve_slot_identifier(slot_id).await
    }

    async fn resolve_batch_identifier(
        &self,
        batch_id: &BatchIdentifier,
    ) -> Result<Option<u64>, Self::Error> {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.resolve_batch_identifier(batch_id).await
    }

    async fn resolve_tx_identifier(
        &self,
        tx_id: &TxIdentifier,
    ) -> Result<Option<u64>, Self::Error> {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.resolve_tx_identifier(tx_id).await
    }

    async fn resolve_event_identifier(
        &self,
        event_id: &EventIdentifier,
    ) -> Result<Option<u64>, Self::Error> {
        let rpc_reader = tokio::task::block_in_place(|| self.get_rpc_reader());
        rpc_reader.resolve_event_identifier(event_id).await
    }

    async fn get_latest_aggregated_proof(&self) -> anyhow::Result<Option<AggregatedProofResponse>> {
        let db = self.db.read().expect(DB_LOCK_POISONED).clone();
        let agg_proof_data = db.get_largest_async::<ProofByUniqueId>().await;

        match agg_proof_data? {
            Some((_, proof)) => Ok(Some(AggregatedProofResponse { proof })),
            None => Ok(None),
        }
    }

    fn subscribe_slots(&self) -> BoxStream<'static, SlotNumber> {
        BroadcastStream::new(self.notification_service.slot_subscriptions.subscribe())
            .filter_map(|data| async move {
                data.map_err(|error| {
                    error!(%error, "Failed to receive slot notification from Tokio channel; this is a bug, please report it");
                })
                .ok()
            })
        .boxed()
    }

    fn subscribe_finalized_slots(&self) -> BoxStream<'static, SlotNumber> {
        WatchStream::new(
            self.notification_service
                .finalized_slot_subscriptions
                .subscribe(),
        )
        .boxed()
    }

    fn subscribe_proof_saved(&self) -> BoxStream<'static, AggregatedProofResponse> {
        BroadcastStream::new(self.notification_service.proof_subscriptions.subscribe())
            .filter_map(|data| async move {
                data.map_err(|error| {
                    error!(%error, "Failed to receive proof notification from Tokio channel; this is a bug, please report it");
                })
                .ok()
            })
            .boxed()
    }
}

impl LedgerDb {
    // This method is blocking via std::sync::RwLock
    pub(crate) fn get_rpc_reader(&self) -> LedgerRpcReader {
        LedgerRpcReader {
            db: self.clone_reader(),
        }
    }
}
