//! RocksDB wrapper for the STF infos of finalized slots, waiting to be proven.

use std::sync::Arc;

use rockbound::{SchemaBatch, DB};
use sov_rollup_interface::common::SlotNumber;

use crate::schema::tables::{StfInfoByNumber, PROOF_MANAGER_TABLES};
use crate::schema::types::StoredStfInfo;
use crate::schema::DeltaReader;
use crate::DbOptions;

/// Database of the proof manager, separate from the `LedgerDb`.
#[derive(Clone, Debug)]
pub struct ProofManagerDb {
    db: Arc<DB>,
}

impl ProofManagerDb {
    const DB_PATH_SUFFIX: &'static str = "proof-manager";
    const DB_NAME: &'static str = "proof-manager-db";

    /// Opens the database in a subdirectory of `path`.
    pub fn open(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let db = DbOptions {
            name: Self::DB_NAME,
            path_suffix: Self::DB_PATH_SUFFIX,
            columns: PROOF_MANAGER_TABLES.to_vec(),
        }
        .default_setup_db_as_subdir(path)?;
        Ok(Self { db: Arc::new(db) })
    }

    /// Gets the STF info of the given slot.
    pub fn get_stf_info(&self, slot: SlotNumber) -> anyhow::Result<Option<StoredStfInfo>> {
        self.db.get::<StfInfoByNumber>(&slot)
    }

    /// Highest slot with a stored STF info, if any.
    pub fn latest_slot(&self) -> anyhow::Result<Option<SlotNumber>> {
        Ok(DeltaReader::new(self.db.clone(), Vec::new())
            .get_largest::<StfInfoByNumber>()?
            .map(|(slot, _)| slot))
    }

    /// Atomically writes `infos` and deletes every STF info below `prune_below`.
    pub fn write_stf_infos(
        &self,
        infos: &[(SlotNumber, StoredStfInfo)],
        prune_below: Option<SlotNumber>,
    ) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::new();
        for (slot, info) in infos {
            batch.put::<StfInfoByNumber>(slot, info)?;
        }
        if let Some(prune_below) = prune_below {
            batch.delete_range::<StfInfoByNumber>(&SlotNumber::GENESIS, &prune_below)?;
        }
        self.db.write_schemas(&batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(slot: u64) -> (SlotNumber, StoredStfInfo) {
        (
            SlotNumber::new(slot),
            StoredStfInfo {
                data: vec![slot as u8; 32],
            },
        )
    }

    #[test]
    fn test_write_prune_and_reopen() {
        let temp_dir = tempfile::tempdir().unwrap();

        {
            let db = ProofManagerDb::open(temp_dir.path()).unwrap();
            assert_eq!(db.latest_slot().unwrap(), None);
            db.write_stf_infos(&[info(1), info(2), info(3)], None)
                .unwrap();
            db.write_stf_infos(&[info(4)], Some(SlotNumber::new(3)))
                .unwrap();
        }

        let db = ProofManagerDb::open(temp_dir.path()).unwrap();
        assert_eq!(db.latest_slot().unwrap(), Some(SlotNumber::new(4)));
        assert_eq!(db.get_stf_info(SlotNumber::new(2)).unwrap(), None);
        assert_eq!(
            db.get_stf_info(SlotNumber::new(3)).unwrap(),
            Some(info(3).1)
        );
    }
}
