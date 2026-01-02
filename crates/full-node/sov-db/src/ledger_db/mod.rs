use std::fmt::Debug;
use std::sync::{Arc, Mutex, RwLock};

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::{Schema, SchemaBatch};
use serde::Serialize;
use sov_rollup_interface::common::{HexHash, SlotNumber};
use sov_rollup_interface::node::da::SlotData;
use sov_rollup_interface::node::ledger_api::AggregatedProofResponse;
use sov_rollup_interface::stf::{BatchReceipt, DiscardedBlob, StoredEvent, TxReceiptContents};
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;

use crate::schema::tables::DiscardedBlobHahsByNumber;
use crate::schema::tables::{
    BatchByHash, BatchByNumber, DiscardedBlobByHash, EventByKey, EventByNumber, FinalizedSlots,
    ProofByUniqueId, SlotByHash, SlotByNumber, StfInfoByNumber, StfInfoMetadata, TxByHash,
    TxByNumber, LEDGER_TABLES,
};
use crate::schema::types::{
    split_tx_for_storage, BatchNumber, DiscardedBlobNumber, EventNumber,
    LatestFinalizedSlotSingleton, ProofUniqueId, StfInfoUniqueId, StoredBatch, StoredDiscardedBlob,
    StoredSlot, StoredStfInfo, StoredTransaction, TxNumber,
};
use crate::DbOptions;

/// Helper functions to query from events.
pub mod event_helper;
mod rpc;
mod rpc_constants;

pub(crate) const DB_LOCK_POISONED: &str = "Internal db lock is poisoned";

/// A RollupHeight, BatchNumber, TxNumber, and EventNumber which are grouped together, typically representing
/// the respective heights at the start or end of slot processing.
#[derive(Default, Clone, Debug)]
#[cfg_attr(feature = "arbitrary", derive(proptest_derive::Arbitrary))]
pub struct ItemNumbers {
    /// The rollup height
    pub slot_number: SlotNumber,
    /// The batch number
    pub batch_number: u64,
    /// The discarded batch number
    pub discarded_batch_number: u64,
    /// The transaction number
    pub tx_number: u64,
    /// The event number
    pub event_number: u64,
}

/// All the data to be committed to the ledger db for a single slot.
#[derive(Debug)]
pub struct SlotCommit<S: SlotData, B, T: TxReceiptContents> {
    slot_data: S,
    batch_receipts: Vec<BatchReceipt<B, T>>,
    discarded_blobs: Vec<DiscardedBlob>,
    num_txs: usize,
    num_events: usize,
}

impl<S: SlotData, B, T: TxReceiptContents> SlotCommit<S, B, T> {
    /// Returns a reference to the commit's slot_data
    pub fn slot_data(&self) -> &S {
        &self.slot_data
    }

    /// Returns a reference to the commit's batch_receipts
    pub fn batch_receipts(&self) -> &[BatchReceipt<B, T>] {
        &self.batch_receipts
    }

    /// Create a new SlotCommit from the given slot data
    pub fn new(slot_data: S, discarded_blobs: Vec<DiscardedBlob>) -> Self {
        Self {
            slot_data,
            batch_receipts: vec![],
            discarded_blobs,
            num_txs: 0,
            num_events: 0,
        }
    }
    /// Add a `batch` (of transactions) to the commit
    pub fn add_batch(&mut self, batch: BatchReceipt<B, T>) {
        self.num_txs += batch.tx_receipts.len();
        let events_this_batch: usize = batch.tx_receipts.iter().map(|r| r.events.len()).sum();
        self.batch_receipts.push(batch);
        self.num_events += events_this_batch;
    }
}

#[derive(Debug, Clone)]
struct Notification<T> {
    triggered_at_slot: SlotNumber,
    notification: T,
}

type NotificationBacklog<T> = Arc<Mutex<Vec<Notification<T>>>>;

/// Single struct responsible for aggregating and sending all notifications.
#[derive(Debug, Clone)]
pub(crate) struct LedgerNotificationService {
    // Regular slots
    slot_notifications: NotificationBacklog<SlotNumber>,
    pub(crate) slot_subscriptions: tokio::sync::broadcast::Sender<SlotNumber>,
    // Finalized slots
    finalized_slot_notifications: NotificationBacklog<SlotNumber>,
    pub(crate) finalized_slot_subscriptions: tokio::sync::watch::Sender<SlotNumber>,
    // Proofs
    proof_notifications: NotificationBacklog<AggregatedProofResponse>,
    pub(crate) proof_subscriptions: tokio::sync::broadcast::Sender<AggregatedProofResponse>,
}

impl LedgerNotificationService {
    pub(crate) fn new() -> Self {
        LedgerNotificationService {
            slot_notifications: Default::default(),
            slot_subscriptions: tokio::sync::broadcast::channel(10).0,
            finalized_slot_notifications: Default::default(),
            finalized_slot_subscriptions: tokio::sync::watch::Sender::new(SlotNumber::GENESIS),
            proof_notifications: Default::default(),
            proof_subscriptions: tokio::sync::broadcast::channel(10).0,
        }
    }

    pub(crate) fn register_slot_notification(
        &self,
        triggered_at_slot: SlotNumber,
        slot_number: SlotNumber,
    ) {
        tracing::trace!(
            %triggered_at_slot,
            finalize_slot = %slot_number,
            "Registering processed slot notification");
        self.slot_notifications
            .lock()
            .expect("Slot notification lock is poisoned")
            .push(Notification {
                triggered_at_slot,
                notification: slot_number,
            });
    }

    pub(crate) fn register_finalized_slot_notification(
        &self,
        triggered_at_slot: SlotNumber,
        slot_number: SlotNumber,
    ) {
        tracing::trace!(
            %triggered_at_slot,
            finalized_slot = %slot_number,
            "Registering finalized slot notification");
        self.finalized_slot_notifications
            .lock()
            .expect("Finalized slot notification lock is poisoned")
            .push(Notification {
                triggered_at_slot,
                notification: slot_number,
            });
    }

    pub(crate) fn register_aggregated_proof_notification(
        &self,
        triggered_at_slot: SlotNumber,
        aggregated_proof: AggregatedProofResponse,
    ) {
        tracing::trace!(
            %triggered_at_slot,
            "Registering aggregated proof notification");
        self.proof_notifications
            .lock()
            .expect("Aggregated proof notification lock is poisoned")
            .push(Notification {
                triggered_at_slot,
                notification: aggregated_proof,
            });
    }

    pub(crate) fn send_notifications_for_slot(&self, slot: SlotNumber) {
        tracing::trace!(slot_number = %slot, "Start sending notifications");
        {
            let mut slot_notifications = self
                .slot_notifications
                .lock()
                .expect("Slot notification lock is poisoned");

            slot_notifications.retain(|slot_num| {
                if slot_num.triggered_at_slot <= slot {
                    let _ = self.slot_subscriptions.send(slot_num.notification);
                    false
                } else {
                    true
                }
            });
        }
        tracing::trace!(slot_number = %slot, "Slot notifications are sent");

        {
            let mut finalized_slot_notifications = self
                .finalized_slot_notifications
                .lock()
                .expect("Finalized slot notification lock is poisoned");

            finalized_slot_notifications.retain(|rollup_height| {
                if rollup_height.triggered_at_slot <= slot {
                    let _ = self
                        .finalized_slot_subscriptions
                        .send(rollup_height.notification);
                    false
                } else {
                    true
                }
            });
        }
        tracing::trace!(slot_number = %slot, "Finalized slot notifications are sent");

        {
            let mut proof_notifications = self
                .proof_notifications
                .lock()
                .expect("Proof notification lock is poisoned");

            proof_notifications.retain(|agg_proof| {
                if agg_proof.triggered_at_slot <= slot {
                    let _ = self
                        .proof_subscriptions
                        .send(agg_proof.notification.clone());
                    false
                } else {
                    true
                }
            });
        }
        tracing::trace!(slot_number = %slot, "All notifications are sent");
    }

    pub(crate) fn send_notifications(&self) {
        self.send_notifications_for_slot(SlotNumber::MAX);
    }
}

#[derive(Clone, Debug)]
/// A database which stores the ledger history (slots, transactions, events, etc.).
/// Ledger data is first ingested into an in-memory map
/// before being fed to the state-transition function.
/// Once the state-transition function has been executed and finalized,
/// the results are committed to the final db
pub struct LedgerDb {
    /// The database which stores the committed ledger.
    /// Uses an optimized layout which
    /// requires transactions to be executed before being committed.
    /// Using [`RwLock`] here, because Ledger is also used as an RPC,
    /// so storage replacement needs to be propagated to other cloned instances.
    db: Arc<RwLock<DeltaReader>>,
    notification_service: LedgerNotificationService,
}

// Db key for the latest height of the written STF info.
const WRITE_ROLLUP_HEIGHT_ID: StfInfoUniqueId = StfInfoUniqueId(0);
// DB key for the latest height of the retrieved STF info.
const NEXT_SLOT_NUMBER_TO_RECEIVE_ID: StfInfoUniqueId = StfInfoUniqueId(1);
// Db key for the oldest saved STF info.
const LAST_SLOT_NUMBER_ID: StfInfoUniqueId = StfInfoUniqueId(2);

impl LedgerDb {
    const DB_PATH_SUFFIX: &'static str = "ledger";
    const DB_NAME: &'static str = "ledger-db";

    /// Create [`DbOptions`] for [`LedgerDb`].
    pub fn get_rockbound_options() -> DbOptions {
        DbOptions {
            name: Self::DB_NAME,
            path_suffix: Self::DB_PATH_SUFFIX,
            columns: LEDGER_TABLES.to_vec(),
        }
    }

    /// Initialize a new [`LedgerDb`] that shares the [`LedgerNotificationService`]
    /// of the provided ledger_db. It uses the other [`LedgerDb`]s inner reader
    /// as **the starting point**.
    ///
    /// This is used to create a [`LedgerDb`] used for serving API requests
    /// and publishing notifications produced by the "core" ledger instance.
    ///
    /// To provide strong consistency across the REST API,
    /// we need to delay updating the inner reader of such ledger dbs until
    /// the full execution has completed (including the sequencer).
    ///
    /// This solves an issue where `/ledger/slots/finalized` would return a slot_number
    /// then that slot number was used to query module state:
    /// `/modules/my-module/state/a?slot_number=slot_number` but would return an invalid
    /// slot number error because state updates hadn't propagated to module-related
    /// state accessors.
    ///
    /// Important note: Produced instance of the [`LedgerDb`] will have a new independent `RwLock`,
    /// so a call to `other.replace_reader()` won't propagate data to the newly created instance.
    /// It is by design.
    pub fn with_shared_notifications(other: &LedgerDb) -> Self {
        Self {
            db: Arc::new(RwLock::new(other.clone_reader())),
            notification_service: other.notification_service.clone(),
        }
    }

    /// Initialize a new [`LedgerDb`] with the provided [`DeltaReader`].
    pub fn with_reader(reader: DeltaReader) -> anyhow::Result<Self> {
        Ok(Self {
            db: Arc::new(RwLock::new(reader)),
            notification_service: LedgerNotificationService::new(),
        })
    }

    /// Replace the underlying reader with a new [`DeltaReader`].
    pub fn replace_reader(&self, reader: DeltaReader) {
        let mut existing_reader = self.db.write().expect(DB_LOCK_POISONED);
        *existing_reader = reader;
    }

    /// Clones the underlying `DeltaReader` used by this `LedgerDb`.
    /// Blocking if called in async context
    pub fn clone_reader(&self) -> DeltaReader {
        self.db.read().expect(DB_LOCK_POISONED).clone()
    }

    /// Get the next slot, block, transaction, and event numbers.
    pub fn get_next_items_numbers(&self) -> anyhow::Result<ItemNumbers> {
        let db = self.db.read().expect(DB_LOCK_POISONED).clone();
        Ok(ItemNumbers {
            slot_number: Self::last_version_written(&db, SlotByNumber)?
                .map(|mut x| x.incr())
                .unwrap_or_default(),
            batch_number: Self::last_version_written(&db, BatchByNumber)?
                .map(|x| x.0 + 1)
                .unwrap_or_default(),
            discarded_batch_number: Self::last_version_written(&db, DiscardedBlobHahsByNumber)?
                .map(|x| x.0 + 1)
                .unwrap_or_default(),
            tx_number: Self::last_version_written(&db, TxByNumber)?
                .map(|x| x.0 + 1)
                .unwrap_or_default(),
            event_number: Self::last_version_written(&db, EventByNumber)?
                .map(|x| x.0 + 1)
                .unwrap_or_default(),
        })
    }

    pub(crate) fn put_slot(
        &self,
        slot: &StoredSlot,
        slot_number: &SlotNumber,
        schema_batch: &mut SchemaBatch,
    ) -> anyhow::Result<()> {
        schema_batch.put::<SlotByNumber>(slot_number, slot)?;
        schema_batch.put::<SlotByHash>(&slot.hash, slot_number)
    }

    fn put_batch(
        &self,
        batch: &StoredBatch,
        batch_number: &BatchNumber,
        schema_batch: &mut SchemaBatch,
    ) -> anyhow::Result<()> {
        schema_batch.put::<BatchByNumber>(batch_number, batch)?;
        schema_batch.put::<BatchByHash>(&batch.hash, batch_number)
    }

    fn put_discarded_blob(
        &self,
        blob: StoredDiscardedBlob,
        discarded_batch_number: DiscardedBlobNumber,
        schema_batch: &mut SchemaBatch,
    ) -> anyhow::Result<()> {
        schema_batch.put::<DiscardedBlobHahsByNumber>(
            &discarded_batch_number,
            &blob.discarded_blob.hash.0,
        )?;
        schema_batch.put::<DiscardedBlobByHash>(&blob.discarded_blob.hash.0, &blob)
    }

    fn put_transaction(
        &self,
        tx: &StoredTransaction,
        tx_number: &TxNumber,
        schema_batch: &mut SchemaBatch,
    ) -> anyhow::Result<()> {
        schema_batch.put::<TxByNumber>(tx_number, tx)?;
        schema_batch.put::<TxByHash>(&(tx.hash, *tx_number), &())
    }

    fn put_event(
        &self,
        event: &StoredEvent,
        event_number: &EventNumber,
        tx_number: TxNumber,
        schema_batch: &mut SchemaBatch,
    ) -> anyhow::Result<()> {
        schema_batch.put::<EventByNumber>(event_number, event)?;
        schema_batch.put::<EventByKey>(&(event.key().clone(), tx_number, *event_number), &())
    }

    /// Materializes [`SlotCommit`] into [`SchemaBatch`] by inserting its events,
    /// transactions, and batches before inserting the slot metadata.
    /// Blocking if called in async context
    pub fn materialize_slot<S: SlotData, B: Serialize, T: TxReceiptContents>(
        &self,
        data_to_commit: SlotCommit<S, B, T>,
        state_root: &[u8],
    ) -> anyhow::Result<SchemaBatch> {
        let mut current_item_numbers = self.get_next_items_numbers()?;
        let mut schema_batch = SchemaBatch::new();

        let slot_number = current_item_numbers.slot_number;

        let first_batch_number = current_item_numbers.batch_number;
        let last_batch_number = first_batch_number + data_to_commit.batch_receipts.len() as u64;
        // Insert data from "bottom up" to ensure consistency if the application crashes during insertion
        for batch_receipt in data_to_commit.batch_receipts.into_iter() {
            let batch_number = BatchNumber(current_item_numbers.batch_number);

            let first_tx_number = current_item_numbers.tx_number;
            let last_tx_number = first_tx_number + batch_receipt.tx_receipts.len() as u64;
            // Insert transactions and events from each batch before inserting the batch
            for tx in batch_receipt.tx_receipts.into_iter() {
                let (tx_to_store, events) =
                    split_tx_for_storage(tx, batch_number, current_item_numbers.event_number);
                for event in events.into_iter() {
                    self.put_event(
                        &event,
                        &EventNumber(current_item_numbers.event_number),
                        TxNumber(current_item_numbers.tx_number),
                        &mut schema_batch,
                    )?;
                    current_item_numbers.event_number += 1;
                }
                self.put_transaction(
                    &tx_to_store,
                    &TxNumber(current_item_numbers.tx_number),
                    &mut schema_batch,
                )?;
                current_item_numbers.tx_number += 1;
            }

            // Insert batch
            let batch_to_store = StoredBatch {
                hash: batch_receipt.batch_hash,
                txs: TxNumber(first_tx_number)..TxNumber(last_tx_number),
                receipt: bincode::serialize(&batch_receipt.inner)
                    .expect("serialization to vec is infallible")
                    .into(),
                slot_number,
            };
            self.put_batch(&batch_to_store, &batch_number, &mut schema_batch)?;
            current_item_numbers.batch_number += 1;
        }

        let first_discarded_blob_number = current_item_numbers.discarded_batch_number;

        let last_discarded_blob_number =
            first_discarded_blob_number + data_to_commit.discarded_blobs.len() as u64;

        for (discarded_blob_index, discarded_blob) in
            data_to_commit.discarded_blobs.into_iter().enumerate()
        {
            let discarded_blob_index = discarded_blob_index as u64;
            self.put_discarded_blob(
                StoredDiscardedBlob {
                    discarded_blob,
                    slot_number,
                },
                DiscardedBlobNumber(first_discarded_blob_number + discarded_blob_index),
                &mut schema_batch,
            )?;
        }

        // Once all batches are inserted, Insert slot
        let slot_to_store = StoredSlot {
            hash: data_to_commit.slot_data.hash(),
            state_root: state_root.to_vec().into(),
            // TODO: Add a method to the slot data trait allowing additional data to be stored
            extra_data: vec![].into(),
            batches: BatchNumber(first_batch_number)..BatchNumber(last_batch_number),
            discarded_blobs: DiscardedBlobNumber(first_discarded_blob_number)
                ..DiscardedBlobNumber(last_discarded_blob_number),
            timestamp: data_to_commit.slot_data.timestamp(),
        };
        self.put_slot(&slot_to_store, &slot_number, &mut schema_batch)?;

        self.notification_service
            .register_slot_notification(slot_number, slot_number);

        Ok(schema_batch)
    }

    /// Sending all previously registered notifications.
    pub fn send_notifications(&self) {
        self.notification_service.send_notifications();
    }

    /// Send all notifications for slots <= `slot_number`.
    pub fn send_notifications_for_slot(&self, slot_number: SlotNumber) {
        self.notification_service
            .send_notifications_for_slot(slot_number);
    }

    /// Materializes the latest finalized slot and registers notification.
    pub fn materialize_latest_finalize_slot(
        &self,
        current_slot_number: SlotNumber,
        finalized_slot_number: SlotNumber,
    ) -> anyhow::Result<SchemaBatch> {
        let mut schema_batch = SchemaBatch::new();
        schema_batch
            .put::<FinalizedSlots>(&LatestFinalizedSlotSingleton, &finalized_slot_number)?;
        // Register notification for the slot number that was finalized. It will get *sent* after the slot is finished committing.
        self.notification_service
            .register_finalized_slot_notification(current_slot_number, finalized_slot_number);
        Ok(schema_batch)
    }

    fn last_version_written<T: Schema<Key = U>, U>(
        db: &DeltaReader,
        _schema: T,
    ) -> anyhow::Result<Option<U>> {
        let largest = db.get_largest::<T>()?;

        match largest {
            Some((k, _v)) => Ok(Some(k)),
            _ => Ok(None),
        }
    }

    /// Get the most recent committed slot, if any.
    pub fn get_head_slot(&self) -> anyhow::Result<Option<(SlotNumber, StoredSlot)>> {
        // Clone immediately, so instance is short-lived, reducing the probability of race-condition
        // according to [`tokio::sync::watch::Receiver::borrow`] documentation
        self.db
            .read()
            .expect(DB_LOCK_POISONED)
            .clone()
            .get_largest::<SlotByNumber>()
    }

    /// Materializes aggregated zk proof
    pub fn materialize_aggregated_proof(
        &self,
        current_slot_number: SlotNumber,
        agg_proof: SerializedAggregatedProof,
    ) -> anyhow::Result<SchemaBatch> {
        let mut schema_batch = SchemaBatch::new();
        let unique_id = 0;
        schema_batch.put::<ProofByUniqueId>(&ProofUniqueId(unique_id), &agg_proof)?;

        self.notification_service
            .register_aggregated_proof_notification(
                current_slot_number,
                AggregatedProofResponse { proof: agg_proof },
            );
        Ok(schema_batch)
    }

    /// Materializes [`StoredStfInfo`] into [`SchemaBatch`].
    pub fn materialize_stf_info(
        &self,
        stf_info: &StoredStfInfo,
        slot_number: SlotNumber,
    ) -> anyhow::Result<SchemaBatch> {
        let mut schema_batch = SchemaBatch::new();
        schema_batch.put::<StfInfoByNumber>(&slot_number, stf_info)?;
        Ok(schema_batch)
    }

    /// Get [`StoredStfInfo`] for the given rollup height.
    pub fn get_stf_info(&self, slot_num: SlotNumber) -> anyhow::Result<Option<StoredStfInfo>> {
        let db = self.db.read().expect(DB_LOCK_POISONED).clone();
        db.get::<StfInfoByNumber>(&slot_num)
    }

    /// Materializes the latest height of the written STF info.
    pub fn materialize_stf_info_write_slot_number(
        &self,
        stf_write_slot_number: SlotNumber,
    ) -> anyhow::Result<SchemaBatch> {
        let mut schema_batch = SchemaBatch::new();
        schema_batch.put::<StfInfoMetadata>(&WRITE_ROLLUP_HEIGHT_ID, &stf_write_slot_number)?;
        Ok(schema_batch)
    }

    /// Gets the latest height of the written STF info.
    pub async fn get_stf_info_write_slot_number(&self) -> anyhow::Result<Option<SlotNumber>> {
        let db = self.db.read().expect(DB_LOCK_POISONED).clone();
        db.get_async::<StfInfoMetadata>(&WRITE_ROLLUP_HEIGHT_ID)
            .await
    }

    /// Materializes the latest height of the retrieved STF info.
    pub fn materialize_stf_info_next_slot_number_to_receive(
        &self,
        read_slot_number: SlotNumber,
    ) -> anyhow::Result<SchemaBatch> {
        let mut schema_batch = SchemaBatch::new();
        schema_batch.put::<StfInfoMetadata>(&NEXT_SLOT_NUMBER_TO_RECEIVE_ID, &read_slot_number)?;
        Ok(schema_batch)
    }

    /// Gets the latest height of the submitted STF info.
    pub async fn get_stf_info_next_slot_number_to_receive(
        &self,
    ) -> anyhow::Result<Option<SlotNumber>> {
        let db = self.db.read().expect(DB_LOCK_POISONED).clone();
        db.get_async::<StfInfoMetadata>(&NEXT_SLOT_NUMBER_TO_RECEIVE_ID)
            .await
    }

    /// Materializes the oldest height of the retrieved STF info.
    pub fn materialize_stf_info_oldest_slot_number(
        &self,
        read_slot_number: SlotNumber,
    ) -> anyhow::Result<SchemaBatch> {
        let mut schema_batch = SchemaBatch::new();
        schema_batch.put::<StfInfoMetadata>(&LAST_SLOT_NUMBER_ID, &read_slot_number)?;
        Ok(schema_batch)
    }

    /// Delete STF info for the given slot number.
    pub fn delete_stf_info(&self, slot_number: SlotNumber) -> anyhow::Result<SchemaBatch> {
        let mut schema_batch = SchemaBatch::new();
        schema_batch.delete::<StfInfoByNumber>(&slot_number)?;
        Ok(schema_batch)
    }

    /// Gets the oldest slot number STF info in the Db.
    pub async fn get_stf_info_oldest_slot_number(&self) -> anyhow::Result<Option<SlotNumber>> {
        let db = self.db.read().expect(DB_LOCK_POISONED).clone();
        db.get_async::<StfInfoMetadata>(&LAST_SLOT_NUMBER_ID).await
    }

    /// Gets the discarded blob (if any) corresponding to the given `blob_hash`.
    pub async fn get_discarded_blob_by_hash(
        &self,
        blob_hash: HexHash,
    ) -> anyhow::Result<Option<StoredDiscardedBlob>> {
        let db = self.db.read().expect(DB_LOCK_POISONED).clone();
        db.get_async::<DiscardedBlobByHash>(&blob_hash.0).await
    }

    /// Geht the head state root hash.
    pub fn get_head_root_hash(db: Arc<rockbound::DB>) -> anyhow::Result<Option<[u8; 64]>> {
        let db = DeltaReader::new(db, Vec::new());
        let state_root = db
            .get_largest::<SlotByNumber>()?
            .map(|(_, stored_slot)| {
                let root = stored_slot.state_root.as_ref();
                root.try_into()
            })
            .transpose()?;

        Ok(state_root)
    }

    /// Rolls back the last committed slot from the ledger database.
    /// If there are no slots in the database, this method returns `Ok(())` without doing anything.
    pub fn rollback_head_slot(ledger_db: Arc<rockbound::DB>) -> anyhow::Result<()> {
        let schema_batch = Self::create_schema_batch_for_rollback(ledger_db.clone())?;
        ledger_db.write_schemas(&schema_batch)?;
        Ok(())
    }

    fn create_schema_batch_for_rollback(db: Arc<rockbound::DB>) -> anyhow::Result<SchemaBatch> {
        // Hold the same lock for the entire duration of this method.

        let mut schema_batch = SchemaBatch::new();
        let db = DeltaReader::new(db, Vec::new());

        // Get the current head slot
        let Some((head_slot_number, head_slot)) = db.get_largest::<SlotByNumber>()? else {
            tracing::debug!("No slots in database, nothing to rollback");
            return Ok(schema_batch);
        };

        tracing::info!(
            slot_number = %head_slot_number,
            "Rolling back last slot from ledger database"
        );

        // Delete all discarded blobs
        for current_discarded_blob_number in
            head_slot.discarded_blobs.start.0..head_slot.discarded_blobs.end.0
        {
            let current_discarded_blob_number = DiscardedBlobNumber(current_discarded_blob_number);
            if let Some(discarded_blob_hash) =
                db.get::<DiscardedBlobHahsByNumber>(&current_discarded_blob_number)?
            {
                Self::delete_discarded_blob(
                    &mut schema_batch,
                    discarded_blob_hash,
                    &current_discarded_blob_number,
                )?;
            }
        }

        // Delete all batches in this slot
        for current_batch_number in head_slot.batches.start.0..head_slot.batches.end.0 {
            let current_batch_number = BatchNumber(current_batch_number);

            if let Some(batch) = db.get::<BatchByNumber>(&current_batch_number)? {
                // Delete all transactions in this batch
                for current_tx_number in batch.txs.start.0..batch.txs.end.0 {
                    let current_tx_number = TxNumber(current_tx_number);

                    if let Some(tx) = db.get::<TxByNumber>(&current_tx_number)? {
                        // Delete all events in this transaction
                        for current_event_number in tx.events.start.0..tx.events.end.0 {
                            let current_event_number = EventNumber(current_event_number);

                            if let Some(event) = db.get::<EventByNumber>(&current_event_number)? {
                                Self::delete_event(
                                    &mut schema_batch,
                                    current_tx_number,
                                    &event,
                                    current_event_number,
                                )?;
                            }
                        }
                        Self::delete_tx(&mut schema_batch, &tx, current_tx_number)?;
                    }
                }

                Self::delete_batch(&mut schema_batch, &batch, &current_batch_number)?;
            }
        }

        Self::delete_slot(&mut schema_batch, &head_slot, &head_slot_number)?;

        // Check if we need to update the finalized slot.
        if let Some(finalized_slot) = db.get::<FinalizedSlots>(&LatestFinalizedSlotSingleton)? {
            if finalized_slot >= head_slot_number {
                // Find the previous slot number.
                let new_finalized_slot = if head_slot_number > SlotNumber::GENESIS {
                    // Get the previous slot
                    let mut prev_slot = head_slot_number;
                    prev_slot.decr();
                    prev_slot
                } else {
                    SlotNumber::GENESIS
                };
                tracing::info!(
                    old_finalized_slot = %finalized_slot,
                    new_finalized_slot = %new_finalized_slot,
                    "Updating finalized slot during rollback"
                );
                schema_batch
                    .put::<FinalizedSlots>(&LatestFinalizedSlotSingleton, &new_finalized_slot)?;
            }
        }

        Ok(schema_batch)
    }

    fn delete_event(
        schema_batch: &mut SchemaBatch,
        tx_number: TxNumber,
        event: &StoredEvent,
        event_number: EventNumber,
    ) -> anyhow::Result<()> {
        schema_batch.delete::<EventByNumber>(&event_number)?;
        schema_batch.delete::<EventByKey>(&(event.key().clone(), tx_number, event_number))?;
        Ok(())
    }

    fn delete_tx(
        schema_batch: &mut SchemaBatch,
        tx: &StoredTransaction,
        tx_number: TxNumber,
    ) -> anyhow::Result<()> {
        schema_batch.delete::<TxByNumber>(&tx_number)?;
        schema_batch.delete::<TxByHash>(&(tx.hash, tx_number))?;
        Ok(())
    }

    fn delete_batch(
        schema_batch: &mut SchemaBatch,
        batch: &StoredBatch,
        batch_number: &BatchNumber,
    ) -> anyhow::Result<()> {
        schema_batch.delete::<BatchByNumber>(batch_number)?;
        schema_batch.delete::<BatchByHash>(&batch.hash)?;
        Ok(())
    }

    fn delete_discarded_blob(
        schema_batch: &mut SchemaBatch,
        discarded_blob_hash: [u8; 32],
        discarded_blob_number: &DiscardedBlobNumber,
    ) -> anyhow::Result<()> {
        schema_batch.delete::<DiscardedBlobHahsByNumber>(discarded_blob_number)?;
        schema_batch.delete::<DiscardedBlobByHash>(&discarded_blob_hash)?;
        Ok(())
    }

    fn delete_slot(
        schema_batch: &mut SchemaBatch,
        slot: &StoredSlot,
        slot_number: &SlotNumber,
    ) -> anyhow::Result<()> {
        schema_batch.delete::<SlotByNumber>(slot_number)?;
        schema_batch.delete::<SlotByHash>(&slot.hash)?;
        Ok(())
    }
}
