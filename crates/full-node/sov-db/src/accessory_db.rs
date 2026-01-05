use std::sync::Arc;

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;
use sov_rollup_interface::common::SlotNumber;

use crate::schema::tables::{AccessoryKeysByVersion, ModuleAccessoryState, ACCESSORY_TABLES};
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
                panic!("fooo");
            }

            // Delete from both the main table and the secondary index
            schema_batch.delete::<ModuleAccessoryState>(&(key.clone(), slot_number))?;
            schema_batch.delete::<AccessoryKeysByVersion>(&(slot_number, key))?;
            keys_deleted += 1;
        }

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
    use std::sync::Arc;

    use sov_rollup_interface::common::IntoSlotNumber;

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
            0.to_slot_number(),
        )
        .unwrap();
        rocksdb.write_schemas(&changes0).unwrap();

        // Write data at version 1
        let key2 = b"key2".to_vec();
        let value2 = b"value2".to_vec();
        let changes1 = AccessoryDb::materialize_values(
            vec![(key2.clone(), Some(value2.clone()))],
            1.to_slot_number(),
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
        assert!(found_keys.contains(&(0.to_slot_number(), key1)));
        assert!(found_keys.contains(&(1.to_slot_number(), key2)));
    }

    #[test]
    fn rollback_single_version() {
        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = Arc::new(
            AccessoryDb::get_rockbound_options()
                .default_setup_db_in_path(tempdir.path())
                .unwrap(),
        );
        let reader = DeltaReader::new(rocksdb.clone(), Vec::new());
        let db = AccessoryDb::with_reader(reader).unwrap();

        // Write data at version 0
        let key1 = b"key1".to_vec();
        let value1 = b"value1".to_vec();
        let changes0 = AccessoryDb::materialize_values(
            vec![(key1.clone(), Some(value1.clone()))],
            0.to_slot_number(),
        )
        .unwrap();
        rocksdb.write_schemas(&changes0).unwrap();

        // Write data at version 1
        let key2 = b"key2".to_vec();
        let value2 = b"value2".to_vec();
        let key3 = b"key3".to_vec();
        let value3 = b"value3".to_vec();
        let changes1 = AccessoryDb::materialize_values(
            vec![
                (key2.clone(), Some(value2.clone())),
                (key3.clone(), Some(value3.clone())),
            ],
            1.to_slot_number(),
        )
        .unwrap();
        rocksdb.write_schemas(&changes1).unwrap();

        // Write data at version 2
        let key4 = b"key4".to_vec();
        let value4 = b"value4".to_vec();
        let changes2 = AccessoryDb::materialize_values(
            vec![(key4.clone(), Some(value4.clone()))],
            2.to_slot_number(),
        )
        .unwrap();
        rocksdb.write_schemas(&changes2).unwrap();

        // Verify all data exists before rollback
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key1), 0.to_slot_number())
                .unwrap(),
            Some(value1.clone())
        );
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key2), 1.to_slot_number())
                .unwrap(),
            Some(value2)
        );
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key3), 1.to_slot_number())
                .unwrap(),
            Some(value3)
        );
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key4), 2.to_slot_number())
                .unwrap(),
            Some(value4.clone())
        );

        // Rollback version 1
        AccessoryDb::rollback_version(rocksdb.clone(), 1.to_slot_number()).unwrap();

        // Verify data at version 0 still exists
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key1), 0.to_slot_number())
                .unwrap(),
            Some(value1)
        );

        // Verify data at version 1 is gone
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key2), 1.to_slot_number())
                .unwrap(),
            None
        );
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key3), 1.to_slot_number())
                .unwrap(),
            None
        );

        // Verify data at version 2 still exists
        assert_eq!(
            db.get_value_option(&SlotKey::from_slice(&key4), 2.to_slot_number())
                .unwrap(),
            Some(value4)
        );
    }
}
