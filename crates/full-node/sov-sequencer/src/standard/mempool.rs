use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::num::NonZero;
use std::ops::Bound;
use std::sync::Arc;

use sov_modules_api::{DaSpec, FullyBakedTx};
use sov_rollup_interface::common::HexString;
use sov_rollup_interface::TxHash;
use tracing::{debug, trace};
use uuid::Uuid;

use crate::{TxStatus, TxStatusManager};

/// ID of a [`MempoolTx`].
pub type MempoolTxId = u128;

/// Wrapper around encoded transactions that is ideal for database storage.
///
/// Transaction hashes are cached together with the transaction itself, and each
/// transaction is assigned a monotonically increasing
/// [UUIDv7](https://en.wikipedia.org/wiki/Universally_unique_identifier#Version_7_(timestamp_and_random)),
/// which is then converted to a [`u128`].
///
/// Note, this is **not** part of the [`Sequencer`] interface and it's just a
/// utility that [`Sequencer`] implementations MAY use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MempoolTx {
    /// The encoded transaction bytes.
    pub tx: FullyBakedTx,
    /// The hash of the transaction, as calculated by
    /// the batch builder.
    pub hash: TxHash,
    /// A monotonically increasing UUIDv7 counter used to order transactions by
    /// insertion time. Gaps are allowed.
    pub uuid_v7: u128,
}

impl MempoolTx {
    /// Creates a new [`MempoolTx`] from the given transaction bytes.
    pub fn new(hash: TxHash, tx: FullyBakedTx) -> Self {
        // UUIDv7 are monotonically increasing. See here:
        // <https://github.com/uuid-rs/uuid/releases/tag/1.9.0>.
        let uuid_v7 = Uuid::now_v7().as_u128();

        trace!(uuid_v7, "Generating a new `MempoolTx`");

        Self { tx, hash, uuid_v7 }
    }
}

#[derive(derivative::Derivative)]
#[derivative(Debug)]
pub struct Mempool<Da: DaSpec> {
    max_txs_count: NonZero<usize>,
    txsm: TxStatusManager<Da>,
    // Transaction data
    // ----------------
    txs_ordered_by_most_fair_fit: BTreeMap<MempoolCursor, Arc<MempoolTx>>,
    txs_ordered_by_incremental_id: BTreeMap<MempoolTxId, Arc<MempoolTx>>,
    txs_by_hash: HashMap<TxHash, Arc<MempoolTx>>,
}

impl<Da: DaSpec> Mempool<Da> {
    /// Creates a new [`Mempool`] with the given capacity and initializes it
    /// with the given transactions.
    pub fn new(txsm: TxStatusManager<Da>, max_txs_count: NonZero<usize>) -> anyhow::Result<Self> {
        Ok(Self {
            max_txs_count,
            txsm,
            txs_ordered_by_incremental_id: BTreeMap::new(),
            txs_ordered_by_most_fair_fit: BTreeMap::new(),
            txs_by_hash: HashMap::new(),
        })
    }

    pub fn len(&self) -> usize {
        let len = self.txs_by_hash.len();
        assert!(len <= self.max_txs_count.get());

        len
    }

    /// Fetches the next transaction to include in the batch, if a suitable one
    /// exists.
    pub fn next(&self, cursor: &mut MempoolCursor) -> Option<Arc<MempoolTx>> {
        let mut iter = self
            .txs_ordered_by_most_fair_fit
            // The lower bound is always ignored, so we don't fetch the last
            // transaction again but we go to the next one.
            .range((Bound::Excluded(*cursor), Bound::Unbounded));
        let (next_cursor, tx) = iter.next()?;

        // Important: we update the cursor so the caller can make another call
        // and get the next transaction.
        *cursor = *next_cursor;

        Some(tx.clone())
    }

    /// Remove the tx from the mempool without notifying subscribers
    pub fn drop_without_notifying(&mut self, hash: &TxHash) {
        let Some(tx) = self.txs_by_hash.remove(hash) else {
            return;
        };

        let cursor = MempoolCursor::from_db_tx(&tx);

        self.txs_ordered_by_incremental_id.remove(&tx.uuid_v7);
        self.txs_ordered_by_most_fair_fit.remove(&cursor);
    }

    /// Drop a transaction from the mempool and notify subscribers.
    pub fn drop_and_notify(&mut self, hash: &TxHash, reason: String) {
        self.drop_without_notifying(hash);
        // Notify about the drop.
        self.txsm.notify(*hash, TxStatus::Dropped { reason });
    }

    fn make_space_for_tx(&mut self) {
        while self.len() >= self.max_txs_count.get() {
            let tx_hash = self
                // We always evict the oldest transaction first.
                .txs_ordered_by_incremental_id
                .first_key_value()
                .expect("Mempool is empty but it doesn't have size zero; this is a bug, please report it")
                .1
                .hash;

            debug!(
                mempool_max_txs_count = self.max_txs_count,
                mempool_current_txs_count = self.len(),
                tx_hash = %HexString::new(tx_hash),
                "Evicting transaction from the mempool to make space for a new one"
            );

            self.drop_and_notify(&tx_hash, "Mempool is full".to_string());
        }
    }

    pub fn add_new_tx(&mut self, hash: TxHash, baked_tx: FullyBakedTx) -> Arc<MempoolTx> {
        if let Some(tx) = self.txs_by_hash.get(&hash) {
            // We already have this transaction in the mempool; simply return a
            // reference to it (don't re-add it!).
            tx.clone()
        } else {
            let tx = Arc::new(MempoolTx::new(hash, baked_tx));
            self.add(tx.clone());
            tx
        }
    }

    pub fn add(&mut self, tx: Arc<MempoolTx>) {
        self.make_space_for_tx();

        let cursor = MempoolCursor::from_db_tx(&tx);

        self.txs_ordered_by_incremental_id
            .insert(tx.uuid_v7, tx.clone());
        self.txs_ordered_by_most_fair_fit.insert(cursor, tx.clone());
        self.txs_by_hash.insert(tx.hash, tx.clone());
    }

    pub fn contains(&self, tx_hash: &TxHash) -> bool {
        self.txs_by_hash.contains_key(tx_hash)
    }
}

/// An opaque cursor for [`FairMempool`] iteration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct MempoolCursor {
    tx_size_in_bytes: usize,
    uuid_v7: MempoolTxId,
}

impl MempoolCursor {
    pub fn new(tx_size_in_bytes: usize) -> Self {
        Self {
            tx_size_in_bytes,
            uuid_v7: 0,
        }
    }

    pub fn from_db_tx(tx: &MempoolTx) -> Self {
        Self {
            tx_size_in_bytes: tx.tx.data.len(),
            uuid_v7: tx.uuid_v7,
        }
    }
}

impl PartialOrd for MempoolCursor {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MempoolCursor {
    fn cmp(&self, other: &Self) -> Ordering {
        // Transactions in the mempool are ordered:
        // 1. from largest to smallest, and
        // 2. by least recent to most recent after that.
        let size_ordering = self.tx_size_in_bytes.cmp(&other.tx_size_in_bytes).reverse();
        let temporal_ordering = self.uuid_v7.cmp(&other.uuid_v7);

        size_ordering.then(temporal_ordering)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_strategy::proptest]
    fn mempool_cursor_ordering_is_correct(
        mc1: MempoolCursor,
        mc2: MempoolCursor,
        mc3: MempoolCursor,
    ) {
        reltester::ord(&mc1, &mc2, &mc3).unwrap();
    }
}
