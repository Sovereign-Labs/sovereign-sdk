use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;
use sov_rollup_interface::common::SlotNumber;

use crate::schema::tables::{ModuleAccessoryState, ACCESSORY_TABLES};
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
        key: &AccessoryKey,
        version: SlotNumber,
    ) -> anyhow::Result<AccessoryStateValue> {
        ensure_version_is_correct(
            key,
            version,
            self.db
                .get_prev::<ModuleAccessoryState>(&(key.to_vec(), version))?,
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
            batch.put::<ModuleAccessoryState>(&(key, version), &value)?;
        }
        Ok(batch)
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
            db.get_value_option(&key, 0.to_slot_number()).unwrap(),
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
            db.get_value_option(&key, 0.to_slot_number()).unwrap(),
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
            db.get_value_option(&key, 0.to_slot_number()).unwrap(),
            Some(value.clone())
        );

        let changes2 =
            AccessoryDb::materialize_values(vec![(key.clone(), None)], 0.to_slot_number()).unwrap();
        rocksdb.write_schemas(&changes2).unwrap();
        assert_eq!(db.get_value_option(&key, 0.to_slot_number()).unwrap(), None);
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
        assert_eq!(db.get_value_option(&key, 0.to_slot_number()).unwrap(), None);
    }
}
