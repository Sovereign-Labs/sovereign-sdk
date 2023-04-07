use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::ensure;
use schemadb::{Schema, DB};
use sovereign_sdk::{
    db::{SeekKeyEncoder, SlotStore},
    spec::RollupSpec,
    stf::Event,
};

use crate::{
    rocks_db_config::gen_rocksdb_options,
    schema::{
        tables::{
            BatchByHash, BatchByNumber, EventByKey, EventByNumber, SlotByHash, SlotByNumber,
            TxByHash, TxByNumber, LEDGER_TABLES,
        },
        types::{
            BatchNumber, EventNumber, SlotNumber, StoredBatch, StoredSlot, StoredTransaction,
            TxNumber,
        },
    },
};

mod rpc;

const LEDGER_DB_PATH_SUFFIX: &'static str = "ledger";

#[derive(Clone)]
/// A database which stores the ledger history (slots, transactions, events, etc).
/// Ledger data is first ingested into an in-memory map before being fed to the state-transition function.
/// Once the state-transition function has been executed and finalzied, the results are committed to the final db
pub struct LedgerDB<R: RollupSpec> {
    /// The RocksDB which stores the committed ledger. Uses an optimized layout which
    /// requires transactions to be executed before being committed.
    db: Arc<DB>,
    /// In memory storage for slots that have not yet been executed.
    slots_to_execute: Arc<Mutex<HashMap<[u8; 32], R::SlotData>>>,
    next_item_numbers: Arc<Mutex<ItemNumbers>>,
}

#[derive(Default, Clone, Debug)]
pub struct ItemNumbers {
    pub slot_number: u64,
    pub batch_number: u64,
    pub tx_number: u64,
    pub event_number: u64,
}

#[derive(Default)]
pub struct SlotCommitBuilder {
    pub slot_data: Option<StoredSlot>,
    pub batches: Vec<StoredBatch>,
    pub txs: Vec<StoredTransaction>,
    pub events: Vec<Vec<Event>>,
}

impl SlotCommitBuilder {
    pub fn finalize(self) -> Result<SlotCommit, anyhow::Error> {
        let commit = SlotCommit {
            slot_data: self.slot_data.ok_or(anyhow::format_err!(
                "Slot data is required to commit a slot."
            ))?,
            batches: self.batches,
            txs: self.txs,
            events: self.events,
        };

        ensure!(
            commit.txs.len() == commit.events.len(),
            "Number of transactions must match number of event groupss."
        );
        Ok(commit)
    }
}

pub struct SlotCommit {
    pub slot_data: StoredSlot,
    pub batches: Vec<StoredBatch>,
    pub txs: Vec<StoredTransaction>,
    pub events: Vec<Vec<Event>>,
}

impl SlotCommitBuilder {}

impl<S: RollupSpec> LedgerDB<S> {
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
            slots_to_execute: Default::default(),
            next_item_numbers: Arc::new(Mutex::new(next_item_numbers)),
        })
    }

    /// A rocksdb instance which stores its data in a tempdir
    #[cfg(any(test, feature = "temp"))]
    pub fn temporary() -> Self {
        let path = schemadb::temppath::TempPath::new();
        Self::with_path(path).unwrap()
    }

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
        let max_items = (range.start.into() - range.end.into()) as usize;
        raw_iter.seek(&range.start)?;
        let iter = raw_iter.take(max_items);
        let mut out = Vec::with_capacity(max_items);
        for res in iter {
            let (_, batch) = res?;
            out.push(batch)
        }
        Ok(out)
    }

    fn put_slot(&self, slot: &StoredSlot, slot_number: &SlotNumber) -> Result<(), anyhow::Error> {
        self.db.put::<SlotByNumber>(slot_number, slot)?;
        self.db.put::<SlotByHash>(&slot.hash, slot_number)
    }

    fn put_batch(
        &self,
        batch: &StoredBatch,
        batch_number: &BatchNumber,
    ) -> Result<(), anyhow::Error> {
        self.db.put::<BatchByNumber>(batch_number, batch)?;
        self.db.put::<BatchByHash>(&batch.hash, batch_number)
    }

    fn put_transaction(
        &self,
        tx: &StoredTransaction,
        tx_number: &TxNumber,
    ) -> Result<(), anyhow::Error> {
        self.db.put::<TxByNumber>(tx_number, tx)?;
        self.db.put::<TxByHash>(&tx.hash, tx_number)
    }

    fn put_event(
        &self,
        event: &Event,
        event_number: &EventNumber,
        tx_number: TxNumber,
    ) -> Result<(), anyhow::Error> {
        self.db.put::<EventByNumber>(event_number, event)?;
        self.db
            .put::<EventByKey>(&(event.key.clone(), tx_number, *event_number), &())
    }

    pub fn commit_slot(&self, data_to_commit: SlotCommit) -> Result<(), anyhow::Error> {
        // Create a scope to ensure that the lock is released before we commit to the db
        let item_numbers = {
            let mut next_item_numbers = self.next_item_numbers.lock().unwrap();
            let item_numbers = next_item_numbers.clone();

            ensure!(
                next_item_numbers.batch_number == data_to_commit.slot_data.batches.start.into(),
                "First batch number must be the next in sequence."
            );
            if let Some(first_batch) = data_to_commit.batches.first() {
                ensure!(
                    next_item_numbers.tx_number == first_batch.txs.start.into(),
                    "first ransaction number must be the next in sequence."
                );
            }
            if let Some(first_tx) = data_to_commit.txs.first() {
                ensure!(
                    next_item_numbers.event_number == first_tx.events.start.into(),
                    "first event number must be the next in sequence."
                );
            }

            next_item_numbers.slot_number += 1;
            next_item_numbers.batch_number += data_to_commit.batches.len() as u64;
            next_item_numbers.tx_number += data_to_commit.txs.len() as u64;
            next_item_numbers.event_number += data_to_commit.events.len() as u64;
            item_numbers
            // The lock is released here
        };

        // Insert data from "bottom up" to ensure consistency is present if the application crashes during insert

        let mut event_number = item_numbers.event_number;
        // Insert transactions and events
        for (idx, (tx, event_group)) in data_to_commit
            .txs
            .into_iter()
            .zip(data_to_commit.events.into_iter())
            .enumerate()
        {
            let tx_number = TxNumber(item_numbers.tx_number + idx as u64);
            for event in event_group.into_iter() {
                self.put_event(&event, &EventNumber(event_number), tx_number)?;
                event_number += 1;
            }
            self.put_transaction(&tx, &tx_number)?;
        }

        // Insert batches
        for (idx, batch) in data_to_commit.batches.into_iter().enumerate() {
            let batch_number = BatchNumber(item_numbers.batch_number + idx as u64);
            self.put_batch(&batch, &batch_number)?;
        }

        // Insert slot
        self.put_slot(
            &data_to_commit.slot_data,
            &SlotNumber(item_numbers.slot_number),
        )?;

        Ok(())
    }

    fn last_version_written<T: Schema<Key = U>, U: Into<u64>>(
        db: &DB,
        _schema: T,
    ) -> anyhow::Result<Option<u64>> {
        let mut iter = db.iter::<T>()?;
        iter.seek_to_last();

        return match iter.next() {
            Some(Ok((version, _))) => Ok(Some(version.into())),
            Some(Err(e)) => Err(e),
            _ => Ok(None),
        };
    }
}

impl<S: RollupSpec> SlotStore for LedgerDB<S> {
    type Slot = S::SlotData;

    fn get(&self, hash: &[u8; 32]) -> Option<Self::Slot> {
        self.slots_to_execute.lock().unwrap().remove(hash)
    }

    fn insert(&self, hash: [u8; 32], slot_data: Self::Slot) {
        self.slots_to_execute
            .lock()
            .unwrap()
            .insert(hash, slot_data);
    }
}
