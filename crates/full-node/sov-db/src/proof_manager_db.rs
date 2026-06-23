//! Implements a wrapper around RocksDB for storing the proof-manager STF-info queue.
//!
//! This database persists only the re-provable `StateTransitionInfo` rows, keyed by
//! `(slot, DA block hash)` so competing forks at the same slot are distinct rows. It holds **no**
//! cursor metadata: `next_height_to_receive`, the finalized-visible cutoff, and the prune lower
//! bound are all in-memory and recomputed on startup from the ledger view plus the stored key
//! range. Fewer persisted values means fewer cross-DB invariants to reconcile after a crash.

use std::num::NonZero;
use std::sync::Arc;

use rockbound::{SchemaBatch, DB};
use sov_rollup_interface::common::SlotNumber;

use crate::schema::tables::{StfInfoByNumber, PROOF_MANAGER_TABLES};
use crate::schema::types::{DbHash, StoredStfInfo};
use crate::schema::DeltaReader;
use crate::DbOptions;

/// Smallest possible DA hash — the low bound of a slot's key range.
const MIN_HASH: DbHash = [u8::MIN; 32];

/// Database for the proof-manager STF-info queue, persisted independently from ledger commits.
#[derive(Clone, Debug)]
pub struct ProofManagerDb {
    db: Arc<DB>,
}

impl ProofManagerDb {
    const DB_PATH_SUFFIX: &'static str = "proof-manager";
    const DB_NAME: &'static str = "proof-manager-db";

    /// Get [`DbOptions`] for [`ProofManagerDb`].
    pub fn get_rockbound_options() -> DbOptions {
        DbOptions {
            name: Self::DB_NAME,
            path_suffix: Self::DB_PATH_SUFFIX,
            columns: PROOF_MANAGER_TABLES.to_vec(),
        }
    }

    /// Create a new [`ProofManagerDb`] from an existing RocksDB instance.
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Open the database at the given path.
    pub fn open(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let db = Self::get_rockbound_options().default_setup_db_as_subdir(path)?;
        Ok(Self::new(Arc::new(db)))
    }

    /// Store STF info for a `(slot, DA block hash)`.
    pub fn put_stf_info(
        &self,
        slot: SlotNumber,
        hash: DbHash,
        info: &StoredStfInfo,
    ) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::new();
        batch.put::<StfInfoByNumber>(&(slot, hash), info)?;
        self.db.write_schemas(&batch)?;
        Ok(())
    }

    /// Get STF info for a specific `(slot, DA block hash)`.
    pub fn get_stf_info(
        &self,
        slot: SlotNumber,
        hash: DbHash,
    ) -> anyhow::Result<Option<StoredStfInfo>> {
        self.db.get::<StfInfoByNumber>(&(slot, hash))
    }

    /// Delete STF info for a specific `(slot, DA block hash)`.
    pub fn delete_stf_info(&self, slot: SlotNumber, hash: DbHash) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::new();
        batch.delete::<StfInfoByNumber>(&(slot, hash))?;
        self.db.write_schemas(&batch)?;
        Ok(())
    }

    /// Oldest slot with any stored STF row (seek-first), if the queue is non-empty.
    pub fn oldest_present_slot(&self) -> anyhow::Result<Option<SlotNumber>> {
        Ok(self
            .reader()
            .get_smallest::<StfInfoByNumber>()?
            .map(|((slot, _hash), _)| slot))
    }

    /// Highest slot with any stored STF row (seek-last), if the queue is non-empty.
    pub fn highest_present_slot(&self) -> anyhow::Result<Option<SlotNumber>> {
        Ok(self
            .reader()
            .get_largest::<StfInfoByNumber>()?
            .map(|((slot, _hash), _)| slot))
    }

    /// Whether any fork row exists at `slot` (across all hashes).
    pub fn has_row_at_slot(&self, slot: SlotNumber) -> anyhow::Result<bool> {
        // `get_prev` returns the largest key <= the seek key; a row exists at `slot` iff that
        // key's slot component equals `slot`. `[u8::MAX; 32]` is the largest hash at the slot.
        Ok(self
            .reader()
            .get_prev::<StfInfoByNumber>(&(slot, [u8::MAX; 32]))?
            .is_some_and(|((found_slot, _hash), _)| found_slot == slot))
    }

    /// Prune every stored STF row strictly below the retention window, sweeping orphan forks.
    ///
    /// Keeps the most recent `max_entries` finalized-visible slots. Never prunes at or above
    /// `next_height_to_receive` (the consumer cursor), so unconsumed slots survive even if the
    /// window would otherwise drop them.
    pub fn prune(
        &self,
        finalized_visible_height: SlotNumber,
        next_height_to_receive: SlotNumber,
        max_entries: NonZero<u64>,
    ) -> anyhow::Result<()> {
        let Some(window_floor) = finalized_visible_height.checked_sub(max_entries.get()) else {
            // Not enough finalized history to prune yet.
            return Ok(());
        };

        if next_height_to_receive < window_floor {
            tracing::warn!(
                %next_height_to_receive,
                %window_floor,
                "State Transition Info is not consumed fast enough; retaining unconsumed slots. Please check that the consumer works."
            );
        }
        let prune_below = window_floor.min(next_height_to_receive);

        let mut batch = SchemaBatch::new();
        batch.delete_range::<StfInfoByNumber>(
            &(SlotNumber::GENESIS, MIN_HASH),
            &(prune_below, MIN_HASH),
        )?;
        self.db.write_schemas(&batch)?;
        Ok(())
    }

    /// Recompute the finalized-visible cutoff from the ledger view, **without persisting any
    /// metadata**.
    ///
    /// Performs the one correctness-critical write: deletes staged rows above `ledger_head`
    /// (post-crash / reorg debris staged before a ledger commit that never landed).
    ///
    /// Returns the finalized-visible height: the highest finalized slot reachable by a contiguous
    /// run of present **canonical** rows from the oldest retained slot, or [`SlotNumber::GENESIS`]
    /// when nothing is visible. `canonical_hash` maps a slot to the ledger's finalized DA hash so
    /// presence is checked against the canonical fork (an orphan row alone is not "present").
    pub fn recompute_visible_state(
        &self,
        ledger_head: SlotNumber,
        latest_finalized_slot_number: SlotNumber,
        canonical_hash: impl Fn(SlotNumber) -> Option<DbHash>,
    ) -> anyhow::Result<SlotNumber> {
        let latest_finalized = latest_finalized_slot_number.min(ledger_head);

        // 1. Drop staged rows the ledger has not committed (every fork above the ledger head).
        if let Some(highest_present) = self.highest_present_slot()? {
            if highest_present > ledger_head {
                if let Some(first_to_remove) = ledger_head.checked_add(1) {
                    let mut batch = SchemaBatch::new();
                    batch.delete_range::<StfInfoByNumber>(
                        &(first_to_remove, MIN_HASH),
                        &(highest_present.saturating_add(1), MIN_HASH),
                    )?;
                    self.db.write_schemas(&batch)?;
                    tracing::warn!(
                        %ledger_head,
                        %highest_present,
                        "Dropped staged STF rows above the ledger head"
                    );
                }
            }
        }

        // 2. Walk contiguous present canonical rows from the oldest retained slot, bounded by the
        //    latest finalized slot.
        let Some(oldest) = self.oldest_present_slot()? else {
            return Ok(SlotNumber::GENESIS);
        };

        let canonical_present = |slot: SlotNumber| -> anyhow::Result<bool> {
            match canonical_hash(slot) {
                Some(hash) => Ok(self.get_stf_info(slot, hash)?.is_some()),
                None => Ok(false),
            }
        };

        if oldest > latest_finalized || !canonical_present(oldest)? {
            return Ok(SlotNumber::GENESIS);
        }
        let mut cutoff = oldest;
        loop {
            let Some(next) = cutoff.checked_add(1) else {
                break;
            };
            if next > latest_finalized || !canonical_present(next)? {
                break;
            }
            cutoff = next;
        }
        Ok(cutoff)
    }

    fn reader(&self) -> DeltaReader {
        DeltaReader::new(self.db.clone(), Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use sov_rollup_interface::common::SlotNumber;

    use super::*;

    fn create_test_db(path: impl AsRef<Path>) -> ProofManagerDb {
        let raw_db = ProofManagerDb::get_rockbound_options()
            .default_setup_db_as_subdir(path)
            .expect("Failed to open ProofManagerDb");
        ProofManagerDb::new(Arc::new(raw_db))
    }

    /// Deterministic canonical hash for a slot, used both when staging rows and as the resolver.
    fn hash_for(slot: u64) -> DbHash {
        [slot as u8; 32]
    }

    fn make_stf_info(slot: u64) -> StoredStfInfo {
        StoredStfInfo {
            data: vec![slot as u8; 32],
        }
    }

    fn put_canonical(db: &ProofManagerDb, slot: u64) {
        db.put_stf_info(SlotNumber::new(slot), hash_for(slot), &make_stf_info(slot))
            .unwrap();
    }

    #[test]
    fn test_stf_info_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        let slot = SlotNumber::new(1);
        let hash = hash_for(1);
        let info = make_stf_info(1);

        // Initially empty.
        assert_eq!(db.get_stf_info(slot, hash).unwrap(), None);

        // Put and get.
        db.put_stf_info(slot, hash, &info).unwrap();
        assert_eq!(db.get_stf_info(slot, hash).unwrap(), Some(info));

        // Delete.
        db.delete_stf_info(slot, hash).unwrap();
        assert_eq!(db.get_stf_info(slot, hash).unwrap(), None);
    }

    #[test]
    fn test_present_slot_helpers_track_key_range() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        assert_eq!(db.oldest_present_slot().unwrap(), None);
        assert_eq!(db.highest_present_slot().unwrap(), None);

        for slot in 3..=7 {
            put_canonical(&db, slot);
        }

        assert_eq!(db.oldest_present_slot().unwrap(), Some(SlotNumber::new(3)));
        assert_eq!(db.highest_present_slot().unwrap(), Some(SlotNumber::new(7)));
        assert!(db.has_row_at_slot(SlotNumber::new(5)).unwrap());
        assert!(!db.has_row_at_slot(SlotNumber::new(2)).unwrap());
        assert!(!db.has_row_at_slot(SlotNumber::new(8)).unwrap());
    }

    #[test]
    fn test_prune_keeps_window_and_respects_consumer_cursor() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        for slot in 1..=5 {
            put_canonical(&db, slot);
        }

        // finalized_visible=5, consumer at 4, keep last 2 → prune below min(5-2, 4) = 3.
        db.prune(
            SlotNumber::new(5),
            SlotNumber::new(4),
            NonZero::new(2).unwrap(),
        )
        .unwrap();

        assert_eq!(db.get_stf_info(SlotNumber::ONE, hash_for(1)).unwrap(), None);
        assert_eq!(
            db.get_stf_info(SlotNumber::new(2), hash_for(2)).unwrap(),
            None
        );
        assert!(db
            .get_stf_info(SlotNumber::new(3), hash_for(3))
            .unwrap()
            .is_some());
        assert_eq!(db.oldest_present_slot().unwrap(), Some(SlotNumber::new(3)));
    }

    #[test]
    fn test_prune_never_drops_unconsumed_slots() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        for slot in 1..=10 {
            put_canonical(&db, slot);
        }

        // Window would prune below 8, but the consumer has only reached slot 2: keep everything
        // from slot 2 up.
        db.prune(
            SlotNumber::new(10),
            SlotNumber::new(2),
            NonZero::new(2).unwrap(),
        )
        .unwrap();

        assert_eq!(db.get_stf_info(SlotNumber::ONE, hash_for(1)).unwrap(), None);
        assert_eq!(db.oldest_present_slot().unwrap(), Some(SlotNumber::new(2)));
    }

    #[test]
    fn test_prune_sweeps_orphan_forks_below_cutoff() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        // Two competing forks at slot 3, plus canonical rows elsewhere.
        db.put_stf_info(SlotNumber::new(3), [0xAA; 32], &make_stf_info(3))
            .unwrap();
        db.put_stf_info(SlotNumber::new(3), [0xBB; 32], &make_stf_info(3))
            .unwrap();
        put_canonical(&db, 4);

        // Prune below min(5-1, 5) = 4 → both slot-3 forks are swept, slot 4 survives.
        db.prune(
            SlotNumber::new(5),
            SlotNumber::new(5),
            NonZero::new(1).unwrap(),
        )
        .unwrap();

        assert_eq!(
            db.get_stf_info(SlotNumber::new(3), [0xAA; 32]).unwrap(),
            None
        );
        assert_eq!(
            db.get_stf_info(SlotNumber::new(3), [0xBB; 32]).unwrap(),
            None
        );
        assert!(db
            .get_stf_info(SlotNumber::new(4), hash_for(4))
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_recompute_empty_db_is_genesis() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        let cutoff = db
            .recompute_visible_state(SlotNumber::new(100), SlotNumber::new(50), |slot| {
                Some(hash_for(slot.get()))
            })
            .unwrap();
        assert_eq!(cutoff, SlotNumber::GENESIS);
    }

    #[test]
    fn test_recompute_walks_contiguous_finalized_rows() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        for slot in 1..=3 {
            put_canonical(&db, slot);
        }

        // ledger_head well above, but only slots <= 2 are finalized → cutoff stops at 2.
        let cutoff = db
            .recompute_visible_state(SlotNumber::new(10), SlotNumber::new(2), |slot| {
                Some(hash_for(slot.get()))
            })
            .unwrap();
        assert_eq!(cutoff, SlotNumber::new(2));
    }

    #[test]
    fn test_recompute_stops_at_first_missing_canonical_row() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        // Hole at slot 3 (1,2,4,5 present); contiguity from oldest stops at 2.
        for slot in [1, 2, 4, 5] {
            put_canonical(&db, slot);
        }

        let cutoff = db
            .recompute_visible_state(SlotNumber::new(10), SlotNumber::new(5), |slot| {
                Some(hash_for(slot.get()))
            })
            .unwrap();
        assert_eq!(cutoff, SlotNumber::new(2));
    }

    #[test]
    fn test_recompute_drops_rows_above_ledger_head() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        for slot in 1..=10 {
            put_canonical(&db, slot);
        }

        // ledger_head=7, finalized=5 → drop staged slots 8..=10, keep 6..=7 hidden, cutoff=5.
        let cutoff = db
            .recompute_visible_state(SlotNumber::new(7), SlotNumber::new(5), |slot| {
                Some(hash_for(slot.get()))
            })
            .unwrap();

        assert_eq!(cutoff, SlotNumber::new(5));
        assert_eq!(
            db.get_stf_info(SlotNumber::new(8), hash_for(8)).unwrap(),
            None
        );
        assert!(db
            .get_stf_info(SlotNumber::new(6), hash_for(6))
            .unwrap()
            .is_some());
        assert!(db
            .get_stf_info(SlotNumber::new(7), hash_for(7))
            .unwrap()
            .is_some());
        assert_eq!(db.highest_present_slot().unwrap(), Some(SlotNumber::new(7)));
    }

    #[test]
    fn test_only_stf_rows_persist_across_restarts() {
        let temp_dir = tempfile::tempdir().unwrap();

        {
            let db = create_test_db(temp_dir.path());
            put_canonical(&db, 5);
        }

        {
            let db = create_test_db(temp_dir.path());
            assert!(db
                .get_stf_info(SlotNumber::new(5), hash_for(5))
                .unwrap()
                .is_some());
            assert_eq!(db.oldest_present_slot().unwrap(), Some(SlotNumber::new(5)));
        }
    }
}
