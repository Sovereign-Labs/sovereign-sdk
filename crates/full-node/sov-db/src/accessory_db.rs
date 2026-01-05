use std::sync::Arc;

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::{rocksdb, SchemaBatch};
use sov_rollup_interface::common::SlotNumber;

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

    /// TODO
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
    pub fn rollback_version(
        accessory_db: Arc<rockbound::DB>,
        version: SlotNumber,
    ) -> anyhow::Result<()> {
        let schema_batch = Self::create_schema_batch_for_rollback(accessory_db.clone(), version)?;
        accessory_db.write_schemas(&schema_batch)?;
        Ok(())
    }

    fn create_schema_batch_for_rollback(
        db: Arc<rockbound::DB>,
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

            // Sanity check - we should only see keys from the target version
            if slot_number != version {
                break;
            }

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
    use std::{collections::HashMap, sync::Arc};

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

    const VERSION_ZERO: SlotNumber = SlotNumber::new(0);
    const VERSION_ONE: SlotNumber = SlotNumber::new(1);
    const VERSION_TWO: SlotNumber = SlotNumber::new(2);
    const VERSION_MAX: SlotNumber = SlotNumber::new(u64::MAX);

    #[test]
    fn secondary_index_populated() {
        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = Arc::new(
            AccessoryDb::get_rockbound_options()
                .default_setup_db_in_path(tempdir.path())
                .unwrap(),
        );

        // Write data at version 0
        let key1 = b"key1".to_vec();
        let value1 = b"value1".to_vec();

        let changes0 = AccessoryDb::materialize_values(
            vec![(key1.clone(), Some(value1.clone()))],
            VERSION_ZERO,
        )
        .unwrap();
        rocksdb.write_schemas(&changes0).unwrap();

        // Write data at version 1
        let key2 = b"key2".to_vec();
        let value2 = b"value2".to_vec();
        let changes1 = AccessoryDb::materialize_values(
            vec![(key2.clone(), Some(value2.clone()))],
            VERSION_ONE,
        )
        .unwrap();
        rocksdb.write_schemas(&changes1).unwrap();

        // Verify secondary index contains the expected entries
        let mut iter = rocksdb.iter::<AccessoryKeysByVersion>().unwrap();
        iter.seek_to_first();

        let mut found_keys = Vec::new();
        for entry_result in iter {
            let entry = entry_result.unwrap();
            let (version, key) = entry.key;
            found_keys.push((version, key));
        }

        // We should have exactly 2 entries in the secondary index
        assert_eq!(found_keys.len(), 2);
        assert!(found_keys.contains(&(VERSION_ZERO, key1)));
        assert!(found_keys.contains(&(VERSION_ONE, key2)));
    }

    struct TestData {
        map: HashMap<SlotNumber, Vec<(AccessoryKey, AccessoryStateValue)>>,
    }

    impl TestData {
        fn new() -> Self {
            let mut map = HashMap::new();
            map.insert(
                VERSION_ZERO,
                vec![(b"key0".to_vec(), Some(b"value0".to_vec()))],
            );
            map.insert(
                VERSION_ONE,
                vec![
                    (b"key1".to_vec(), Some(b"value1".to_vec())),
                    (b"key11".to_vec(), Some(b"value11".to_vec())),
                ],
            );

            map.insert(
                VERSION_TWO,
                vec![
                    (b"key2".to_vec(), Some(b"value12".to_vec())),
                    (b"key11".to_vec(), Some(b"value12".to_vec())),
                    (b"key0".to_vec(), None),
                ],
            );

            Self { map }
        }

        fn for_version(&self, version: SlotNumber) -> Vec<(AccessoryKey, AccessoryStateValue)> {
            self.map.get(&version).unwrap().clone()
        }
    }

    fn commit(
        accessory_db: &rockbound::DB,
        accessory_bach: &SchemaBatch,
        root_hash: Vec<u8>,
        version: SlotNumber,
    ) -> anyhow::Result<()> {
        let mut root_hash_batch = SchemaBatch::default();
        root_hash_batch
            .put::<StateRootHashes>(&(version, STATE_ROOT_HASH_SINGLETON), &root_hash)?;

        AccessoryDb::commit(accessory_db, accessory_bach, &root_hash_batch)
    }

    #[test]
    fn rollback_version() {
        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = Arc::new(
            AccessoryDb::get_rockbound_options()
                .default_setup_db_in_path(tempdir.path())
                .unwrap(),
        );
        let reader = DeltaReader::new(rocksdb.clone(), Vec::new());
        let db = AccessoryDb::with_reader(reader).unwrap();

        let data = TestData::new();

        // Write data at version 0
        let changes =
            AccessoryDb::materialize_values(data.for_version(VERSION_ZERO), VERSION_ZERO).unwrap();
        //rocksdb.write_schemas(&changes).unwrap();

        commit(&rocksdb, &changes, [0u8; 64].to_vec(), VERSION_ZERO).unwrap();

        // Write data at version 1
        let changes =
            AccessoryDb::materialize_values(data.for_version(VERSION_ONE), VERSION_ONE).unwrap();
        //rocksdb.write_schemas(&changes).unwrap();

        commit(&rocksdb, &changes, [1u8; 64].to_vec(), VERSION_ONE).unwrap();

        // Write data at version 2
        let changes =
            AccessoryDb::materialize_values(data.for_version(VERSION_TWO), VERSION_TWO).unwrap();
        //rocksdb.write_schemas(&changes).unwrap();

        commit(&rocksdb, &changes, [2u8; 64].to_vec(), VERSION_TWO).unwrap();

        for (k, v) in data.for_version(VERSION_TWO) {
            assert_eq!(
                db.get_value_option(&SlotKey::from_slice(&k), VERSION_MAX)
                    .unwrap(),
                v
            );
        }

        // Rollback version 2
        AccessoryDb::rollback_version(rocksdb.clone(), VERSION_TWO).unwrap();

        for (k, v) in data.for_version(VERSION_ONE) {
            assert_eq!(
                db.get_value_option(&SlotKey::from_slice(&k), VERSION_MAX)
                    .unwrap(),
                v
            );
        }

        // Rollback version 1
        AccessoryDb::rollback_version(rocksdb.clone(), VERSION_ONE).unwrap();

        for (k, v) in data.for_version(VERSION_ZERO) {
            assert_eq!(
                db.get_value_option(&SlotKey::from_slice(&k), VERSION_MAX)
                    .unwrap(),
                v
            );
        }
    }
}
