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
        let db = Self::get_rockbound_options().default_setup_db_in_path(path)?;
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

    /// Set the write height (latest STF info written). Writes immediately to disk.
    pub fn set_write_height(&self, slot: SlotNumber) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::new();
        batch.put::<StfInfoMetadata>(&WRITE_ROLLUP_HEIGHT_ID, &slot)?;
        self.db.write_schemas(&batch)?;
        Ok(())
    }

    /// Get the write height (latest STF info written).
    pub fn get_write_height(&self) -> anyhow::Result<Option<SlotNumber>> {
        self.db.get::<StfInfoMetadata>(&WRITE_ROLLUP_HEIGHT_ID)
    }

    /// Set next_height_to_receive. Writes immediately to disk.
    ///
    /// This is the critical method that fixes the duplicate proof submission bug.
    /// Call this immediately after successful aggregated proof posting.
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

    /// Create a SchemaBatch for setting next_height_to_receive (for batched operations).
    pub fn materialize_next_height_to_receive(
        &self,
        slot: SlotNumber,
    ) -> anyhow::Result<SchemaBatch> {
        let mut batch = SchemaBatch::new();
        batch.put::<StfInfoMetadata>(&NEXT_SLOT_NUMBER_TO_RECEIVE_ID, &slot)?;
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

    /// Validate ProofManagerDb state against the ledger head on startup and
    /// recover `write_height` if metadata lagged behind actual stored STF info.
    ///
    /// Enforces the invariant: proof_manager_write_height <= ledger_head.
    /// If the invariant is violated, return an error to fail fast.
    pub fn validate_and_recover_write_height(&self, ledger_head: SlotNumber) -> anyhow::Result<()> {
        let maybe_write_height = self.get_write_height()?;
        let write_height = maybe_write_height.unwrap_or(SlotNumber::GENESIS);

        if write_height > ledger_head {
            anyhow::bail!(
                "ProofManagerDb ahead of LedgerDb. This is a bug. ledger_head={ledger_head}, proof_manager_write_height={write_height}"
            );
        }

        // Recover write_height if metadata lagged (e.g., crash after staging STF info
        // but before updating metadata). We only advance while STF info is contiguous.
        let mut current = write_height;
        let mut advanced = false;
        loop {
            let Some(next) = current.checked_add(1) else {
                break;
            };
            if next > ledger_head {
                break;
            }
            if self.get_stf_info(next)?.is_some() {
                current = next;
                advanced = true;
            } else {
                break;
            }
        }

        if advanced {
            self.set_write_height(current)?;
            tracing::info!(
                recovered_write_height = %current,
                "Recovered ProofManagerDb write_height from stored STF info"
            );
        } else if maybe_write_height.is_none() {
            tracing::debug!("ProofManagerDb has no metadata to recover");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db(path: impl AsRef<std::path::Path>) -> ProofManagerDb {
        ProofManagerDb::open(path).expect("Failed to open ProofManagerDb")
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
        batch.merge(
            db.materialize_next_height_to_receive(SlotNumber::ONE)
                .unwrap(),
        );

        // Write the batch
        db.write_batch(&batch).unwrap();

        // Verify all operations took effect
        assert!(db.get_stf_info(slot).unwrap().is_some());
        assert_eq!(db.get_write_height().unwrap(), Some(slot));
        assert_eq!(
            db.get_next_height_to_receive().unwrap(),
            Some(SlotNumber::ONE)
        );
    }

    #[test]
    fn test_validate_and_recover_empty_db() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        // Empty db should validate without issues
        db.validate_and_recover_write_height(SlotNumber::new(100))
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
        db.validate_and_recover_write_height(ledger_head).unwrap();

        // State should remain unchanged
        assert_eq!(db.get_write_height().unwrap(), Some(write_height));
    }

    #[test]
    fn test_validate_errors_when_ahead() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = create_test_db(temp_dir.path());

        db.set_write_height(SlotNumber::new(10)).unwrap();
        let err = db
            .validate_and_recover_write_height(SlotNumber::new(7))
            .unwrap_err();
        assert!(err.to_string().contains("ProofManagerDb ahead of LedgerDb"));
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
        db.validate_and_recover_write_height(SlotNumber::new(10))
            .unwrap();

        // write_height should advance to the last contiguous slot
        assert_eq!(db.get_write_height().unwrap(), Some(SlotNumber::new(3)));
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
