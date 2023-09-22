use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use sov_rollup_interface::services::da::SlotData;
use sov_rollup_interface::stf::{BatchReceipt, Event};
use sov_schema_db::{Schema, SchemaBatch, SeekKeyEncoder, DB};

use crate::rocks_db_config::gen_rocksdb_options;
use crate::schema::tables::{
    BatchByHash, BatchByNumber, EventByKey, EventByNumber, SlotByHash, SlotByNumber, TxByHash,
    TxByNumber, LEDGER_TABLES,
};
use crate::schema::types::{
    split_tx_for_storage, BatchNumber, EventNumber, SlotNumber, StoredBatch, StoredSlot,
    StoredTransaction, TxNumber,
};

mod rpc;

const LEDGER_DB_PATH_SUFFIX: &str = "ledger";

#[derive(Clone, Debug)]
/// A database which stores the ledger history (slots, transactions, events, etc).
/// Ledger data is first ingested into an in-memory map before being fed to the state-transition function.
/// Once the state-transition function has been executed and finalzied, the results are committed to the final db
pub struct LedgerDB {
    /// The database which stores the committed ledger. Uses an optimized layout which
    /// requires transactions to be executed before being committed.
    db: Arc<DB>,
    next_item_numbers: Arc<Mutex<ItemNumbers>>,
    slot_subscriptions: tokio::sync::broadcast::Sender<u64>,
}

/// A SlotNumber, BatchNumber, TxNumber, and EventNumber which are grouped together, typically representing
/// the respective heights at the start or end of slot processing.
#[derive(Default, Clone, Debug)]
#[cfg_attr(feature = "arbitrary", derive(proptest_derive::Arbitrary))]
pub struct ItemNumbers {
    /// The slot number
    pub slot_number: u64,
    /// The batch number
    pub batch_number: u64,
    /// The transaction number
    pub tx_number: u64,
    /// The event number
    pub event_number: u64,
}

/// All of the data to be commited to the ledger db for a single slot.
#[derive(Debug)]
pub struct SlotCommit<S: SlotData, B, T> {
    slot_data: S,
    batch_receipts: Vec<BatchReceipt<B, T>>,
    num_txs: usize,
    num_events: usize,
}

impl<S: SlotData, B, T> SlotCommit<S, B, T> {
    /// Returns a reference to the commit's slot_data
    pub fn slot_data(&self) -> &S {
        &self.slot_data
    }

    /// Returns a reference to the commit's batch_receipts
    pub fn batch_receipts(&self) -> &[BatchReceipt<B, T>] {
        &self.batch_receipts
    }

    /// Create a new SlotCommit from the given slot data
    pub fn new(slot_data: S) -> Self {
        Self {
            slot_data,
            batch_receipts: vec![],
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

impl LedgerDB {
    /// Open a [`LedgerDB`] (backed by RocksDB) at the specified path.
    /// The returned instance will be at the path `{path}/ledger-db`.
    pub fn with_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let path = path.as_ref().join(LEDGER_DB_PATH_SUFFIX);
        let inner = DB::open(
            path,
            "ledger-db",
            LEDGER_TABLES.iter().copied(),
            &gen_rocksdb_options(&Default::default(), false),
        )?;

        let next_item_numbers = ItemNumbers {
            slot_number: Self::last_version_written(&inner, SlotByNumber)?.unwrap_or_default() + 1,
            batch_number: Self::last_version_written(&inner, BatchByNumber)?.unwrap_or_default()
                + 1,
            tx_number: Self::last_version_written(&inner, TxByNumber)?.unwrap_or_default() + 1,
            event_number: Self::last_version_written(&inner, EventByNumber)?.unwrap_or_default()
                + 1,
        };

        Ok(Self {
            db: Arc::new(inner),
            next_item_numbers: Arc::new(Mutex::new(next_item_numbers)),
            slot_subscriptions: tokio::sync::broadcast::channel(10).0,
        })
    }

    /// Get the next slot, block, transaction, and event numbers
    pub fn get_next_items_numbers(&self) -> ItemNumbers {
        self.next_item_numbers.lock().unwrap().clone()
    }

    /// Gets all slots with numbers `range.start` to `range.end`. If `range.end` is outside
    /// the range of the database, the result will smaller than the requested range.
    /// Note that this method blindly preallocates for the requested range, so it should not be exposed
    /// directly via rpc.
    pub(crate) fn _get_slot_range(
        &self,
        range: &std::ops::Range<SlotNumber>,
    ) -> Result<Vec<StoredSlot>, anyhow::Error> {
        self.get_data_range::<SlotByNumber, _, _>(range)
    }

    /// Gets all batches with numbers `range.start` to `range.end`. If `range.end` is outside
    /// the range of the database, the result will smaller than the requested range.
    /// Note that this method blindly preallocates for the requested range, so it should not be exposed
    /// directly via rpc.
    pub(crate) fn get_batch_range(
        &self,
        range: &std::ops::Range<BatchNumber>,
    ) -> Result<Vec<StoredBatch>, anyhow::Error> {
        self.get_data_range::<BatchByNumber, _, _>(range)
    }

    /// Gets all transactions with numbers `range.start` to `range.end`. If `range.end` is outside
    /// the range of the database, the result will smaller than the requested range.
    /// Note that this method blindly preallocates for the requested range, so it should not be exposed
    /// directly via rpc.
    pub(crate) fn get_tx_range(
        &self,
        range: &std::ops::Range<TxNumber>,
    ) -> Result<Vec<StoredTransaction>, anyhow::Error> {
        self.get_data_range::<TxByNumber, _, _>(range)
    }

    /// Gets all data with identifier in `range.start` to `range.end`. If `range.end` is outside
    /// the range of the database, the result will smaller than the requested range.
    /// Note that this method blindly preallocates for the requested range, so it should not be exposed
    /// directly via rpc.
    fn get_data_range<T, K, V>(&self, range: &std::ops::Range<K>) -> Result<Vec<V>, anyhow::Error>
    where
        T: Schema<Key = K, Value = V>,
        K: Into<u64> + Copy + SeekKeyEncoder<T>,
    {
        let mut raw_iter = self.db.iter()?;
        let max_items = (range.end.into() - range.start.into()) as usize;
        raw_iter.seek(&range.start)?;
        let iter = raw_iter.take(max_items);
        let mut out = Vec::with_capacity(max_items);
        for res in iter {
            let (_, batch) = res?;
            out.push(batch)
        }
        Ok(out)
    }

    fn put_slot(
        &self,
        slot: &StoredSlot,
        slot_number: &SlotNumber,
        schema_batch: &mut SchemaBatch,
    ) -> Result<(), anyhow::Error> {
        schema_batch.put::<SlotByNumber>(slot_number, slot)?;
        schema_batch.put::<SlotByHash>(&slot.hash, slot_number)
    }

    fn put_batch(
        &self,
        batch: &StoredBatch,
        batch_number: &BatchNumber,
        schema_batch: &mut SchemaBatch,
    ) -> Result<(), anyhow::Error> {
        schema_batch.put::<BatchByNumber>(batch_number, batch)?;
        schema_batch.put::<BatchByHash>(&batch.hash, batch_number)
    }

    fn put_transaction(
        &self,
        tx: &StoredTransaction,
        tx_number: &TxNumber,
        schema_batch: &mut SchemaBatch,
    ) -> Result<(), anyhow::Error> {
        schema_batch.put::<TxByNumber>(tx_number, tx)?;
        schema_batch.put::<TxByHash>(&tx.hash, tx_number)
    }

    fn put_event(
        &self,
        event: &Event,
        event_number: &EventNumber,
        tx_number: TxNumber,
        schema_batch: &mut SchemaBatch,
    ) -> Result<(), anyhow::Error> {
        schema_batch.put::<EventByNumber>(event_number, event)?;
        schema_batch.put::<EventByKey>(&(event.key().clone(), tx_number, *event_number), &())
    }

    /// Commits a slot to the database by inserting its events, transactions, and batches before
    /// inserting the slot metadata.
    pub fn commit_slot<S: SlotData, B: Serialize, T: Serialize>(
        &self,
        data_to_commit: SlotCommit<S, B, T>,
    ) -> Result<(), anyhow::Error> {
        // Create a scope to ensure that the lock is released before we commit to the db
        let mut current_item_numbers = {
            let mut next_item_numbers = self.next_item_numbers.lock().unwrap();
            let item_numbers = next_item_numbers.clone();
            next_item_numbers.slot_number += 1;
            next_item_numbers.batch_number += data_to_commit.batch_receipts.len() as u64;
            next_item_numbers.tx_number += data_to_commit.num_txs as u64;
            next_item_numbers.event_number += data_to_commit.num_events as u64;
            item_numbers
            // The lock is released here
        };

        let mut schema_batch = SchemaBatch::new();

        let first_batch_number = current_item_numbers.batch_number;
        let last_batch_number = first_batch_number + data_to_commit.batch_receipts.len() as u64;
        // Insert data from "bottom up" to ensure consistency if the application crashes during insertion
        for batch_receipt in data_to_commit.batch_receipts.into_iter() {
            let first_tx_number = current_item_numbers.tx_number;
            let last_tx_number = first_tx_number + batch_receipt.tx_receipts.len() as u64;
            // Insert transactions and events from each batch before inserting the batch
            for tx in batch_receipt.tx_receipts.into_iter() {
                let (tx_to_store, events) =
                    split_tx_for_storage(tx, current_item_numbers.event_number);
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
                custom_receipt: bincode::serialize(&batch_receipt.inner)
                    .expect("serialization to vec is infallible")
                    .into(),
            };
            self.put_batch(
                &batch_to_store,
                &BatchNumber(current_item_numbers.batch_number),
                &mut schema_batch,
            )?;
            current_item_numbers.batch_number += 1;
        }

        // Once all batches are inserted, Insert slot
        let slot_to_store = StoredSlot {
            hash: data_to_commit.slot_data.hash(),
            // TODO: Add a method to the slotdata trait allowing additional data to be stored
            extra_data: vec![].into(),
            batches: BatchNumber(first_batch_number)..BatchNumber(last_batch_number),
        };
        self.put_slot(
            &slot_to_store,
            &SlotNumber(current_item_numbers.slot_number),
            &mut schema_batch,
        )?;

        self.db.write_schemas(schema_batch)?;

        // Notify subscribers. This call returns an error IFF there are no subscribers, so we don't need to check the result
        let _ = self
            .slot_subscriptions
            .send(current_item_numbers.slot_number);

        Ok(())
    }

    fn last_version_written<T: Schema<Key = U>, U: Into<u64>>(
        db: &DB,
        _schema: T,
    ) -> anyhow::Result<Option<u64>> {
        let mut iter = db.iter::<T>()?;
        iter.seek_to_last();

        match iter.next() {
            Some(Ok((version, _))) => Ok(Some(version.into())),
            Some(Err(e)) => Err(e),
            _ => Ok(None),
        }
    }

    /// Get the most recent committed slot, if any
    pub fn get_head_slot(&self) -> anyhow::Result<Option<(SlotNumber, StoredSlot)>> {
        let mut iter = self.db.iter::<SlotByNumber>()?;
        iter.seek_to_last();

        match iter.next() {
            Some(Ok((slot_number, slot))) => Ok(Some((slot_number, slot))),
            Some(Err(e)) => Err(e),
            _ => Ok(None),
        }
    }
}

#[cfg(feature = "arbitrary")]
pub mod arbitrary {
    //! Arbitrary definitions for the [`LedgerDB`].

    use core::ops::{Deref, DerefMut};

    use proptest::strategy::LazyJust;
    use tempfile::TempDir;

    use super::*;

    /// Arbitrary instance of [`LedgerDB`].
    ///
    /// This is a db wrapper bound to its temporary path that will be deleted once the type is
    /// dropped.
    #[derive(Debug)]
    pub struct ArbitraryLedgerDB {
        /// The underlying RocksDB instance.
        pub db: LedgerDB,
        /// The temporary directory used to create the [`LedgerDB`].
        pub path: TempDir,
    }

    /// A fallible, arbitrary instance of [`LedgerDB`].
    ///
    /// This type is suitable for operations that are expected to be infallible. The internal
    /// implementation of the db requires I/O to create the temporary dir, making it fallible.
    #[derive(Debug)]
    pub struct FallibleArbitraryLedgerDB {
        /// The result of the new db instance.
        pub result: anyhow::Result<ArbitraryLedgerDB>,
    }

    impl Deref for ArbitraryLedgerDB {
        type Target = LedgerDB;

        fn deref(&self) -> &Self::Target {
            &self.db
        }
    }

    impl DerefMut for ArbitraryLedgerDB {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.db
        }
    }

    impl<'a> ::arbitrary::Arbitrary<'a> for ArbitraryLedgerDB {
        fn arbitrary(_u: &mut ::arbitrary::Unstructured<'a>) -> ::arbitrary::Result<Self> {
            let path = TempDir::new().map_err(|_| ::arbitrary::Error::NotEnoughData)?;
            let db = LedgerDB::with_path(&path).map_err(|_| ::arbitrary::Error::IncorrectFormat)?;
            Ok(Self { db, path })
        }
    }

    impl proptest::arbitrary::Arbitrary for FallibleArbitraryLedgerDB {
        type Parameters = ();
        type Strategy = LazyJust<Self, fn() -> FallibleArbitraryLedgerDB>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            fn gen() -> FallibleArbitraryLedgerDB {
                FallibleArbitraryLedgerDB {
                    result: TempDir::new()
                        .map_err(|e| {
                            anyhow::anyhow!(format!("failed to generate path for LedgerDB: {e}"))
                        })
                        .and_then(|path| {
                            let db = LedgerDB::with_path(&path)?;
                            Ok(ArbitraryLedgerDB { db, path })
                        }),
                }
            }
            LazyJust::new(gen)
        }
    }

    impl<'a> ::arbitrary::Arbitrary<'a> for ItemNumbers {
        fn arbitrary(u: &mut ::arbitrary::Unstructured<'a>) -> ::arbitrary::Result<Self> {
            Ok(ItemNumbers {
                slot_number: u.arbitrary()?,
                batch_number: u.arbitrary()?,
                tx_number: u.arbitrary()?,
                event_number: u.arbitrary()?,
            })
        }
    }
}
