use std::sync::Arc;

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::{rocksdb, SchemaBatch};
use sov_rollup_interface::common::{IntoSlotNumber, SlotNumber};

use crate::historical_state::STATE_ROOT_HASH_SINGLETON;
use crate::schema::tables::{
    AccessoryKeysByVersion, ModuleAccessoryState, StateRootHashes, ACCESSORY_TABLES,
};
use crate::schema::types::slot_key::SlotKey;
use crate::schema::types::{AccessoryKey, AccessoryStateValue};
use crate::{ensure_version_is_correct, DbOptions};

/// Specifies a particular version of the Accessory state.
pub type Version = u64;

/// Typesafe transformer for data, that is not part of the provable state.
#[derive(Clone, Debug)]
pub struct AccessoryDb {
    /// Pointer to [`DeltaReader`] for correct data.
    db: DeltaReader,
}

impl AccessoryDb {
    const DB_PATH_SUFFIX: &'static str = "accessory";
    const DB_NAME: &'static str = "accessory-db";

    /// Get [`DbOptions`] for [`AccessoryDb`]
    pub fn get_rockbound_options() -> DbOptions {
        DbOptions {
            name: Self::DB_NAME,
            path_suffix: Self::DB_PATH_SUFFIX,
            columns: ACCESSORY_TABLES.to_vec(),
        }
    }

    /// Create instance of [`AccessoryDb`] from [`DeltaReader`].
    pub fn with_reader(reader: DeltaReader) -> anyhow::Result<Self> {
        Ok(Self { db: reader })
    }

    /// Queries for a value in the [`AccessoryDb`], given a key.
    pub fn get_value_option(
        &self,
        key: &SlotKey,
        version: SlotNumber,
    ) -> anyhow::Result<AccessoryStateValue> {
        ensure_version_is_correct(
            key.as_ref(),
            version,
            self.db
                .get_prev::<ModuleAccessoryState>(&(key.as_ref().to_vec(), version))?,
        )
    }

    /// Collects a sequence of key-value pairs into [`SchemaBatch`].
    pub fn materialize_values(
        key_value_pairs: impl IntoIterator<Item = (AccessoryKey, AccessoryStateValue)>,
        version: SlotNumber,
    ) -> anyhow::Result<SchemaBatch> {
        let mut batch = SchemaBatch::default();
        for (key, value) in key_value_pairs {
            // We always .put and not .delete to keep archival data.
            batch.put::<ModuleAccessoryState>(&(key.clone(), version), &value)?;
            // Also update the secondary index for efficient rollback
            batch.put::<AccessoryKeysByVersion>(&(version, key), &())?;
        }
        Ok(batch)
    }

    /// Latest state root hash from archival db.
    pub fn latest_version_and_root_hash_archival_db(
        accessory_db: Arc<rockbound::DB>,
    ) -> anyhow::Result<Option<(u64, [u8; 64])>> {
        let reader = DeltaReader::new(accessory_db, Vec::new());
        let latest = reader.get_largest::<StateRootHashes>()?;

        match latest {
            Some(((version, _), root_hash)) => {
                let root_hash_array: [u8; 64] =
                    root_hash.try_into().expect("Root hash muts be [u8; 64]");
                Ok(Some((version.get(), root_hash_array)))
            }
            None => Ok(None),
        }
    }

    /// Write the accessory data to disc.
    pub fn commit(
        accessory_db: &rockbound::DB,
        accessory_bach: &SchemaBatch,
        root_hash_batch: &SchemaBatch,
    ) -> anyhow::Result<()> {
        let mut accessory_db_batch = rocksdb::WriteBatch::default();
        rockbound::DB::update_db_batch_with_schema_data(
            &mut accessory_db_batch,
            accessory_bach,
            accessory_db,
        )?;

        rockbound::DB::update_db_batch_with_schema_data(
            &mut accessory_db_batch,
            root_hash_batch,
            accessory_db,
        )?;

        accessory_db.write_db_batch(accessory_db_batch)?;

        Ok(())
    }

    /// Rollback a specific version of the AccessoryDb.
    /// This will delete all key-value pairs that were written at the specified version.
    pub fn rollback(accessory_db: Arc<rockbound::DB>) -> anyhow::Result<()> {
        let Some((version, _)) =
            Self::latest_version_and_root_hash_archival_db(accessory_db.clone())?
        else {
            return Ok(());
        };

        let schema_batch =
            Self::create_schema_batch_for_rollback(&accessory_db, version.to_slot_number())?;
        accessory_db.write_schemas(&schema_batch)?;

        Ok(())
    }

    fn create_schema_batch_for_rollback(
        db: &rockbound::DB,
        version: SlotNumber,
    ) -> anyhow::Result<SchemaBatch> {
        let mut schema_batch = SchemaBatch::new();

        // Use the secondary index to efficiently find all keys at this version
        // Create a range that covers all keys for this version
        // Since keys are encoded as (version, key), all entries for a version
        // will be between (version, empty) and (version+1, empty)
        let start = (version, Vec::new());

        let mut next_version = version;
        next_version.incr();

        let end = (next_version, Vec::new());

        let mut iter = db.iter_range::<AccessoryKeysByVersion>(&start, &end)?;
        iter.seek_to_first();

        let mut keys_deleted = 0;
        for entry_result in iter {
            let entry = entry_result?;
            let (slot_number, key) = entry.key;
            assert_eq!(slot_number, version);

            // Delete from both the main table and the secondary index
            schema_batch.delete::<ModuleAccessoryState>(&(key.clone(), slot_number))?;
            schema_batch.delete::<AccessoryKeysByVersion>(&(slot_number, key))?;
            keys_deleted += 1;
        }

        // Also delete the StateRootHashes entry for this version
        schema_batch.delete::<StateRootHashes>(&(version, STATE_ROOT_HASH_SINGLETON))?;

        tracing::info!(
            version = %version,
            keys_deleted = keys_deleted,
            "Rolled back version from accessory database"
        );

        Ok(schema_batch)
    }
}

#[cfg(test)]
mod tests {
    use sov_rollup_interface::common::IntoSlotNumber;
    use std::{collections::HashMap, sync::Arc, u64};

    use crate::rocks_db_config;

    use super::*;

    #[test]
    fn get_after_set() {
        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = Arc::new(
            AccessoryDb::get_rockbound_options()
                .default_setup_db_in_path(tempdir.path())
                .unwrap(),
        );
        let reader = DeltaReader::new(rocksdb.clone(), Vec::new());
        let db = AccessoryDb::with_reader(reader).unwrap();

        let key = b"foo".to_vec();
        let value = b"bar".to_vec();
        let changes1 = AccessoryDb::materialize_values(
            vec![(key.clone(), Some(value.clone()))],
            0.to_slot_number(),
        )
        .unwrap();
        rocksdb.write_schemas(&changes1).unwrap();
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key), 0.to_slot_number())
                .unwrap(),
            Some(value.clone())
        );

        let value2 = b"baz".to_vec();
        let changes2 = AccessoryDb::materialize_values(
            vec![(key.clone(), Some(value2.clone()))],
            1.to_slot_number(),
        )
        .unwrap();
        rocksdb.write_schemas(&changes2).unwrap();
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key), 0.to_slot_number())
                .unwrap(),
            Some(value)
        );
    }

    #[test]
    fn get_after_delete() {
        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = Arc::new(
            AccessoryDb::get_rockbound_options()
                .default_setup_db_in_path(tempdir.path())
                .unwrap(),
        );
        let reader = DeltaReader::new(rocksdb.clone(), Vec::new());
        let db = AccessoryDb::with_reader(reader).unwrap();

        let key = b"deleted".to_vec();
        let value = b"baz".to_vec();
        let changes1 = AccessoryDb::materialize_values(
            vec![(key.clone(), Some(value.clone()))],
            0.to_slot_number(),
        )
        .unwrap();
        rocksdb.write_schemas(&changes1).unwrap();
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key), 0.to_slot_number())
                .unwrap(),
            Some(value.clone())
        );

        let changes2 =
            AccessoryDb::materialize_values(vec![(key.clone(), None)], 0.to_slot_number()).unwrap();
        rocksdb.write_schemas(&changes2).unwrap();
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key), 0.to_slot_number())
                .unwrap(),
            None
        );
    }

    #[test]
    fn get_nonexistent() {
        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = Arc::new(
            AccessoryDb::get_rockbound_options()
                .default_setup_db_in_path(tempdir.path())
                .unwrap(),
        );
        let reader = DeltaReader::new(rocksdb.clone(), Vec::new());
        let db = AccessoryDb::with_reader(reader).unwrap();

        let key = b"spam".to_vec();
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key), 0.to_slot_number())
                .unwrap(),
            None
        );
    }

    // This test commits slot 3 and rolls back slots 2 and 1.
    #[test]
    fn rollback_accessory() {
        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = Arc::new(
            AccessoryDb::get_rockbound_options()
                .default_setup_db_in_path(tempdir.path())
                .unwrap(),
        );
        let reader = DeltaReader::new(rocksdb.clone(), Vec::new());
        let db = AccessoryDb::with_reader(reader).unwrap();

        let data = TestData::new();

        // Commit version 0
        {
            let version = 0;
            let changes = AccessoryDb::materialize_values(
                data.for_version(version),
                version.to_slot_number(),
            )
            .unwrap();
            commit(&rocksdb, &changes, version).unwrap();
        }

        // Commit version 1
        {
            let version = 1;
            let changes = AccessoryDb::materialize_values(
                data.for_version(version),
                version.to_slot_number(),
            )
            .unwrap();
            commit(&rocksdb, &changes, version).unwrap();
        }

        // Commit version 2
        {
            let version = 2;
            let changes = AccessoryDb::materialize_values(
                data.for_version(version),
                version.to_slot_number(),
            )
            .unwrap();

            commit(&rocksdb, &changes, version).unwrap();
            data.check_if_reverted(version, &db);
        }

        // Rollback version 2
        {
            AccessoryDb::rollback(rocksdb.clone()).unwrap();
            let version = 1;

            let latest =
                AccessoryDb::latest_version_and_root_hash_archival_db(rocksdb.clone()).unwrap();
            assert_eq!(latest, Some((version, root_hash_from_version(version))));

            data.check_if_reverted(version, &db);
        }

        // Rollback version 1
        {
            AccessoryDb::rollback(rocksdb.clone()).unwrap();
            let version = 0;

            let latest =
                AccessoryDb::latest_version_and_root_hash_archival_db(rocksdb.clone()).unwrap();
            assert_eq!(latest, Some((version, root_hash_from_version(version))));

            data.check_if_reverted(version, &db);
        }
    }

    fn root_hash_from_version(version: u64) -> [u8; 64] {
        [version as u8; 64]
    }

    struct TestData {
        map: HashMap<u64, Vec<(AccessoryKey, AccessoryStateValue)>>,
    }

    impl TestData {
        fn new() -> Self {
            let mut map = HashMap::new();
            map.insert(0, vec![(b"key0".to_vec(), Some(b"value0".to_vec()))]);
            map.insert(
                1,
                vec![
                    (b"key1".to_vec(), Some(b"value1".to_vec())),
                    (b"key11".to_vec(), Some(b"value11".to_vec())),
                ],
            );

            map.insert(
                2,
                vec![
                    (b"key2".to_vec(), Some(b"value12".to_vec())),
                    (b"key11".to_vec(), Some(b"value12".to_vec())),
                    (b"key0".to_vec(), None),
                ],
            );

            Self { map }
        }

        fn check_if_reverted(&self, version: u64, db: &AccessoryDb) {
            for (k, v) in self.for_version(version) {
                assert_eq!(
                    db.get_value_option(&SlotKey::from_slice(&k), u64::MAX.to_slot_number())
                        .unwrap(),
                    v
                );
            }
        }

        fn for_version(&self, version: u64) -> Vec<(AccessoryKey, AccessoryStateValue)> {
            self.map.get(&version).unwrap().clone()
        }
    }

    fn commit(
        accessory_db: &rockbound::DB,
        accessory_bach: &SchemaBatch,
        version: u64,
    ) -> anyhow::Result<()> {
        let root_hash = root_hash_from_version(version).to_vec();
        let version = SlotNumber::new(u64::from(version));
        let mut root_hash_batch = SchemaBatch::default();
        root_hash_batch
            .put::<StateRootHashes>(&(version, STATE_ROOT_HASH_SINGLETON), &root_hash)?;

        AccessoryDb::commit(accessory_db, accessory_bach, &root_hash_batch)
    }
}
