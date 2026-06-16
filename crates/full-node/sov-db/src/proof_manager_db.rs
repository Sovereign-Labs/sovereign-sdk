//! Implements a wrapper around RocksDB for storing proof manager state.
//!
//! This database persists proof-manager-specific data independently from the
//! ledger commit loop, enabling immediate persistence of critical metadata
//! like `next_height_to_receive` after successful aggregated proof posting.

use std::sync::Arc;

use rockbound::{SchemaBatch, DB};
use sov_rollup_interface::common::SlotNumber;

use crate::schema::tables::{StfInfoByNumber, StfInfoMetadata, PROOF_MANAGER_TABLES};
use crate::schema::types::{StfInfoUniqueId, StoredStfInfo};
use crate::schema::DeltaReader;
use crate::DbOptions;

/// DB key for the latest height of the written STF info.
const WRITE_ROLLUP_HEIGHT_ID: StfInfoUniqueId = StfInfoUniqueId(0);
/// DB key for the next height to be received/processed by the proof manager.
const NEXT_SLOT_NUMBER_TO_RECEIVE_ID: StfInfoUniqueId = StfInfoUniqueId(1);
/// DB key for the oldest saved STF info (used for pruning).
const OLDEST_SLOT_NUMBER_ID: StfInfoUniqueId = StfInfoUniqueId(2);
/// Database for proof manager state that persists independently from ledger commits.
///
/// This allows critical proof manager metadata (like `next_height_to_receive`) to be
/// persisted immediately after successful operations, without waiting for the next
/// ledger commit. This fixes issues where restarts could cause duplicate proof
/// submissions.
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

    // ==================== STF Info Operations ====================

    /// Store STF info for a slot. Writes immediately to disk.
    pub fn put_stf_info(&self, slot: SlotNumber, info: &StoredStfInfo) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::new();
        batch.put::<StfInfoByNumber>(&slot, info)?;
        self.db.write_schemas(&batch)?;
        Ok(())
    }

    /// Get STF info for a specific slot.
    pub fn get_stf_info(&self, slot: SlotNumber) -> anyhow::Result<Option<StoredStfInfo>> {
        self.db.get::<StfInfoByNumber>(&slot)
    }

    /// Delete STF info for a slot. Writes immediately to disk.
    pub fn delete_stf_info(&self, slot: SlotNumber) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::new();
        batch.delete::<StfInfoByNumber>(&slot)?;
        self.db.write_schemas(&batch)?;
        Ok(())
    }

    /// Create a SchemaBatch for deleting STF info (for batched operations).
    pub fn materialize_delete_stf_info(&self, slot: SlotNumber) -> anyhow::Result<SchemaBatch> {
        let mut batch = SchemaBatch::new();
        batch.delete::<StfInfoByNumber>(&slot)?;
        Ok(batch)
    }

    // ==================== Metadata Operations ====================

    /// Set the write height (highest finalized STF slot visible to proof-manager consumers).
    ///
    /// STF rows for later non-finalized slots may already be staged in the DB, but they must
    /// remain hidden until ledger finality reaches them.
    pub fn set_write_height(&self, slot: SlotNumber) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::new();
        batch.put::<StfInfoMetadata>(&WRITE_ROLLUP_HEIGHT_ID, &slot)?;
        self.db.write_schemas(&batch)?;
        Ok(())
    }

    /// Get the write height (highest finalized STF slot visible to proof-manager consumers).
    pub fn get_write_height(&self) -> anyhow::Result<Option<SlotNumber>> {
        self.db.get::<StfInfoMetadata>(&WRITE_ROLLUP_HEIGHT_ID)
    }

    /// Set next_height_to_receive. Writes immediately to disk.
    ///
    /// Immediate persistence is critical: if this update waited for the next
    /// ledger commit, a crash after proof posting but before the commit would
    /// cause the node to re-submit the same aggregated proof on restart.
    pub fn set_next_height_to_receive(&self, slot: SlotNumber) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::new();
        batch.put::<StfInfoMetadata>(&NEXT_SLOT_NUMBER_TO_RECEIVE_ID, &slot)?;
        self.db.write_schemas(&batch)?;
        tracing::trace!(%slot, "Persisted next_height_to_receive immediately");
        Ok(())
    }

    /// Get the next height to receive.
    pub fn get_next_height_to_receive(&self) -> anyhow::Result<Option<SlotNumber>> {
        self.db
            .get::<StfInfoMetadata>(&NEXT_SLOT_NUMBER_TO_RECEIVE_ID)
    }

    /// Set the oldest height. Writes immediately to disk.
    pub fn set_oldest_height(&self, slot: SlotNumber) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::new();
        batch.put::<StfInfoMetadata>(&OLDEST_SLOT_NUMBER_ID, &slot)?;
        self.db.write_schemas(&batch)?;
        Ok(())
    }

    /// Get the oldest height.
    pub fn get_oldest_height(&self) -> anyhow::Result<Option<SlotNumber>> {
        self.db.get::<StfInfoMetadata>(&OLDEST_SLOT_NUMBER_ID)
    }

    // ==================== Batch Operations ====================

    /// Create a SchemaBatch for putting STF info (for batched operations).
    pub fn materialize_stf_info(
        &self,
        slot: SlotNumber,
        info: &StoredStfInfo,
    ) -> anyhow::Result<SchemaBatch> {
        let mut batch = SchemaBatch::new();
        batch.put::<StfInfoByNumber>(&slot, info)?;
        Ok(batch)
    }

    /// Create a SchemaBatch for setting write height (for batched operations).
    pub fn materialize_write_height(&self, slot: SlotNumber) -> anyhow::Result<SchemaBatch> {
        let mut batch = SchemaBatch::new();
        batch.put::<StfInfoMetadata>(&WRITE_ROLLUP_HEIGHT_ID, &slot)?;
        Ok(batch)
    }

    /// Create a SchemaBatch for setting oldest height (for batched operations).
    pub fn materialize_oldest_height(&self, slot: SlotNumber) -> anyhow::Result<SchemaBatch> {
        let mut batch = SchemaBatch::new();
        batch.put::<StfInfoMetadata>(&OLDEST_SLOT_NUMBER_ID, &slot)?;
        Ok(batch)
    }

    /// Write a SchemaBatch to the database.
    pub fn write_batch(&self, batch: &SchemaBatch) -> anyhow::Result<()> {
        self.db.write_schemas(batch)?;
        Ok(())
    }

    // ==================== Startup Validation ====================

    /// Validate ProofManagerDb state against the ledger view on startup and
    /// recover finalized-visible `write_height` if metadata lagged behind stored STF info.
    ///
    /// Enforces the invariants:
    /// - proof-manager STF rows must not exist above `ledger_head`
    /// - `write_height` must not advance above `latest_finalized_slot_number`
    ///
    /// Staged STF rows between `latest_finalized_slot_number + 1` and `ledger_head` are preserved
    /// but remain hidden from `notify()` until those slots finalize.
    pub fn validate_and_recover_write_height(
        &self,
        ledger_head: SlotNumber,
        latest_finalized_slot_number: SlotNumber,
    ) -> anyhow::Result<()> {
        let latest_finalized_slot_number = latest_finalized_slot_number.min(ledger_head);
        let maybe_write_height = self.get_write_height()?;
        let mut write_height = maybe_write_height.unwrap_or(SlotNumber::GENESIS);
        let maybe_next_height_to_receive = self.get_next_height_to_receive()?;
        let maybe_oldest_height = self.get_oldest_height()?;
        let max_next_height = ledger_head.saturating_add(1);
        let proof_manager_reader = DeltaReader::new(self.db.clone(), Vec::new());
        let highest_stored_slot = proof_manager_reader
            .get_largest::<StfInfoByNumber>()?
            .map(|v| v.0);
        let highest_retained_slot = match highest_stored_slot {
            Some(highest_stored_slot) if highest_stored_slot > ledger_head => {
                self.find_largest_stored_slot_at_or_below(ledger_head)?
            }
            Some(highest_stored_slot) => Some(highest_stored_slot),
            None => None,
        };
        let mut batch = SchemaBatch::new();
        let mut batch_changed = false;

        // Remove staged rows beyond the current ledger head. This must use the largest stored slot,
        // not `write_height`, because non-finalized rows are staged ahead of finalized visibility.
        if let Some(highest_stored_slot) = highest_stored_slot {
            if let Some(first_to_remove) = ledger_head.checked_add(1) {
                if highest_stored_slot >= first_to_remove {
                    for slot in first_to_remove.range_inclusive(highest_stored_slot) {
                        batch.delete::<StfInfoByNumber>(&slot)?;
                    }
                    batch_changed = true;
                }
            }
        }

        if write_height > latest_finalized_slot_number {
            let old_write_height = write_height;
            batch.put::<StfInfoMetadata>(&WRITE_ROLLUP_HEIGHT_ID, &latest_finalized_slot_number)?;
            batch_changed = true;
            write_height = latest_finalized_slot_number;

            tracing::warn!(
                %ledger_head,
                %latest_finalized_slot_number,
                old_proof_manager_write_height = %old_write_height,
                "Clamped ProofManagerDb write_height to the latest finalized slot"
            );
        }

        let min_next_height = if highest_retained_slot.is_some() {
            maybe_oldest_height.unwrap_or(SlotNumber::ONE)
        } else {
            write_height.saturating_add(1)
        };
        let clamped_next_height = match maybe_next_height_to_receive {
            Some(next_height) => Some(next_height.max(min_next_height).min(max_next_height)),
            None if maybe_write_height.is_some() => Some(min_next_height.min(max_next_height)),
            None => None,
        };
        if clamped_next_height != maybe_next_height_to_receive {
            if let Some(clamped_next_height) = clamped_next_height {
                batch.put::<StfInfoMetadata>(
                    &NEXT_SLOT_NUMBER_TO_RECEIVE_ID,
                    &clamped_next_height,
                )?;
                batch_changed = true;
            }
        }

        let min_oldest_height = if highest_retained_slot.is_some() {
            maybe_oldest_height.unwrap_or(SlotNumber::ONE)
        } else {
            max_next_height
        };
        let clamped_oldest_height = match maybe_oldest_height {
            Some(oldest_height) => Some(oldest_height.max(min_oldest_height).min(max_next_height)),
            None if maybe_write_height.is_some() => Some(min_oldest_height),
            None => None,
        };
        if clamped_oldest_height != maybe_oldest_height {
            if let Some(clamped_oldest_height) = clamped_oldest_height {
                batch.put::<StfInfoMetadata>(&OLDEST_SLOT_NUMBER_ID, &clamped_oldest_height)?;
                batch_changed = true;
            }
        }

        if batch_changed {
            self.write_batch(&batch)?;
        }

        // Recover write_height forward if metadata lagged behind already-finalized STF info.
        match self.recover_contiguous_write_height(write_height, latest_finalized_slot_number)? {
            Some(recovered_write_height) => {
                self.set_write_height(recovered_write_height)?;
                tracing::info!(
                    %recovered_write_height,
                    "Recovered ProofManagerDb write_height from stored STF info"
                );
            }
            None if maybe_write_height.is_none() => {
                tracing::debug!("ProofManagerDb has no metadata to recover");
            }
            None => {}
        }

        Ok(())
    }

    /// Advances `write_height` over contiguous STF rows that the ledger has already finalized.
    ///
    /// Starting just above `from`, walks forward while each next slot is both at-or-below
    /// `latest_finalized_slot_number` and present in the DB. Returns the new (higher) write
    /// height, or `None` if nothing could be advanced.
    fn recover_contiguous_write_height(
        &self,
        from: SlotNumber,
        latest_finalized_slot_number: SlotNumber,
    ) -> anyhow::Result<Option<SlotNumber>> {
        let mut current = from;
        let mut advanced = false;
        loop {
            let Some(next) = current.checked_add(1) else {
                break;
            };
            if next > latest_finalized_slot_number {
                break;
            }
            if self.get_stf_info(next)?.is_some() {
                current = next;
                advanced = true;
            } else {
                break;
            }
        }

        Ok(advanced.then_some(current))
    }

    fn find_largest_stored_slot_at_or_below(
        &self,
        upper_bound: SlotNumber,
    ) -> anyhow::Result<Option<SlotNumber>> {
        let proof_manager_reader = DeltaReader::new(self.db.clone(), Vec::new());
        Ok(proof_manager_reader
            .get_prev::<StfInfoByNumber>(&upper_bound)?
            .map(|(slot, _)| slot))
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

    fn make_stf_info(slot: u64) -> StoredStfInfo {
        StoredStfInfo {
            data: vec![slot as u8; 32],
        }
    }

    #[test]
    fn test_stf_info_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        let slot = SlotNumber::new(1);
        let info = make_stf_info(1);

        // Initially empty
        assert!(db.get_stf_info(slot).unwrap().is_none());

        // Put and get
        db.put_stf_info(slot, &info).unwrap();
        let retrieved = db.get_stf_info(slot).unwrap().unwrap();
        assert_eq!(retrieved.data, info.data);

        // Delete
        db.delete_stf_info(slot).unwrap();
        assert!(db.get_stf_info(slot).unwrap().is_none());
    }

    #[test]
    fn test_metadata_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        // Initially all metadata is None
        assert!(db.get_write_height().unwrap().is_none());
        assert!(db.get_next_height_to_receive().unwrap().is_none());
        assert!(db.get_oldest_height().unwrap().is_none());

        // Set and get write height
        let slot = SlotNumber::new(10);
        db.set_write_height(slot).unwrap();
        assert_eq!(db.get_write_height().unwrap(), Some(slot));

        // Set and get next_height_to_receive
        let next_slot = SlotNumber::new(5);
        db.set_next_height_to_receive(next_slot).unwrap();
        assert_eq!(db.get_next_height_to_receive().unwrap(), Some(next_slot));

        // Set and get oldest height
        let oldest_slot = SlotNumber::new(3);
        db.set_oldest_height(oldest_slot).unwrap();
        assert_eq!(db.get_oldest_height().unwrap(), Some(oldest_slot));
    }

    #[test]
    fn test_batch_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        let slot = SlotNumber::new(1);
        let info = make_stf_info(1);

        // Build a batch with multiple operations
        let mut batch = db.materialize_stf_info(slot, &info).unwrap();
        batch.merge(db.materialize_write_height(slot).unwrap());
        batch.merge(db.materialize_oldest_height(SlotNumber::ONE).unwrap());

        // Write the batch
        db.write_batch(&batch).unwrap();

        // Verify all operations took effect
        assert!(db.get_stf_info(slot).unwrap().is_some());
        assert_eq!(db.get_write_height().unwrap(), Some(slot));
        assert_eq!(db.get_oldest_height().unwrap(), Some(SlotNumber::ONE));
    }

    #[test]
    fn test_validate_and_recover_empty_db() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        // Empty db should validate without issues
        db.validate_and_recover_write_height(SlotNumber::new(100), SlotNumber::new(50))
            .unwrap();
    }

    #[test]
    fn test_validate_and_recover_consistent_state() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        // Set up ProofManagerDb with write_height <= ledger_head
        let write_height = SlotNumber::new(50);
        db.set_write_height(write_height).unwrap();
        db.set_next_height_to_receive(SlotNumber::new(10)).unwrap();

        // Validate with ledger_head >= write_height
        let ledger_head = SlotNumber::new(100);
        db.validate_and_recover_write_height(ledger_head, write_height)
            .unwrap();

        // State should remain unchanged
        assert_eq!(db.get_write_height().unwrap(), Some(write_height));
    }

    #[test]
    fn test_validate_clamps_write_height_to_latest_finalized() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        for slot in 1..=10 {
            let slot = SlotNumber::new(slot);
            db.put_stf_info(slot, &make_stf_info(slot.get())).unwrap();
        }
        db.set_write_height(SlotNumber::new(10)).unwrap();
        db.set_next_height_to_receive(SlotNumber::new(12)).unwrap();

        db.validate_and_recover_write_height(SlotNumber::new(7), SlotNumber::new(5))
            .unwrap();

        assert_eq!(db.get_write_height().unwrap(), Some(SlotNumber::new(5)));
        assert!(db.get_stf_info(SlotNumber::new(8)).unwrap().is_none());
        assert!(db.get_stf_info(SlotNumber::new(6)).unwrap().is_some());
        assert!(db.get_stf_info(SlotNumber::new(7)).unwrap().is_some());
        assert_eq!(
            db.get_next_height_to_receive().unwrap(),
            Some(SlotNumber::new(8))
        );
    }

    #[test]
    fn test_validate_and_recover_advances_write_height() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        // Stage STF info for slots 1..=3
        for slot in 1..=3 {
            let s = SlotNumber::new(slot);
            db.put_stf_info(s, &make_stf_info(slot)).unwrap();
        }

        // Metadata lags behind (write_height is None)
        db.validate_and_recover_write_height(SlotNumber::new(10), SlotNumber::new(2))
            .unwrap();

        // write_height should advance only to the last contiguous finalized slot.
        assert_eq!(db.get_write_height().unwrap(), Some(SlotNumber::new(2)));
    }

    #[test]
    fn test_persistence_across_restarts() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Write data with first instance
        {
            let db = create_test_db(temp_dir.path());
            let slot = SlotNumber::new(5);
            db.put_stf_info(slot, &make_stf_info(5)).unwrap();
            db.set_write_height(slot).unwrap();
            db.set_next_height_to_receive(SlotNumber::new(3)).unwrap();
        }

        // Read data with second instance (simulating restart)
        {
            let db = create_test_db(temp_dir.path());
            assert!(db.get_stf_info(SlotNumber::new(5)).unwrap().is_some());
            assert_eq!(db.get_write_height().unwrap(), Some(SlotNumber::new(5)));
            assert_eq!(
                db.get_next_height_to_receive().unwrap(),
                Some(SlotNumber::new(3))
            );
        }
    }
}
