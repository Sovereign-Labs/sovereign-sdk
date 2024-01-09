use std::path::Path;
use std::sync::Arc;

use sov_schema_db::cache::cache_db::CacheDb;
use sov_schema_db::cache::change_set::ChangeSet;
use sov_schema_db::SchemaBatch;

use crate::rocks_db_config::gen_rocksdb_options;
use crate::schema::tables::{ModuleAccessoryState, NATIVE_TABLES};
use crate::schema::types::AccessoryKey;

/// Specifies a particular version of the Accessory state.
pub type Version = u64;

/// Typesafe wrapper for Data, that is not part of the provable state
/// TODO: Rename to AccessoryDb
#[derive(Debug)]
pub struct NativeDB {
    /// Pointer to [`CacheDb`] for up to date state
    db: Arc<CacheDb>,
}

impl Clone for NativeDB {
    fn clone(&self) -> Self {
        NativeDB {
            db: self.db.clone(),
        }
    }
}

impl NativeDB {
    const DB_PATH_SUFFIX: &'static str = "native-db";
    const DB_NAME: &'static str = "native";

    /// Initialize [`sov_schema_db::DB`] that matches tables and columns for NativeDB
    pub fn setup_schema_db(path: impl AsRef<Path>) -> anyhow::Result<sov_schema_db::DB> {
        let path = path.as_ref().join(Self::DB_PATH_SUFFIX);
        sov_schema_db::DB::open(
            path,
            Self::DB_NAME,
            NATIVE_TABLES.iter().copied(),
            &gen_rocksdb_options(&Default::default(), false),
        )
    }

    /// Convert it to [`ChangeSet`] which cannot be edited anymore
    pub fn freeze(self) -> anyhow::Result<ChangeSet> {
        let inner = Arc::into_inner(self.db).ok_or(anyhow::anyhow!(
            "NativeDB underlying DbSnapshot has more than 1 strong references"
        ))?;
        Ok(ChangeSet::from(inner))
    }

    /// Create instance of [`NativeDB`] from [`CacheDb`]
    pub fn with_db_snapshot(db_snapshot: CacheDb) -> anyhow::Result<Self> {
        // We keep Result type, just for future archival state integration
        Ok(Self {
            db: Arc::new(db_snapshot),
        })
    }

    /// Queries for a value in the [`NativeDB`], given a key.
    pub fn get_value_option(
        &self,
        key: &AccessoryKey,
        version: Version,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let found = self
            .db
            .get_prev::<ModuleAccessoryState>(&(key.to_vec(), version))?;
        match found {
            Some(((found_key, found_version), value)) => {
                if &found_key == key {
                    anyhow::ensure!(found_version <= version, "Bug! iterator isn't returning expected values. expected a version <= {version:} but found {found_version:}");
                    Ok(value)
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    /// Sets a sequence of key-value pairs in the [`NativeDB`]. The write is atomic.
    pub fn set_values(
        &self,
        key_value_pairs: impl IntoIterator<Item = (Vec<u8>, Option<Vec<u8>>)>,
        version: Version,
    ) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::default();
        for (key, value) in key_value_pairs {
            batch.put::<ModuleAccessoryState>(&(key, version), &value)?;
        }
        self.db.write_many(batch)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::RwLock;

    use sov_schema_db::cache::{NoopQueryManager, ReadOnlyLock};

    use super::*;

    fn setup_db() -> NativeDB {
        let manager = ReadOnlyLock::new(Arc::new(RwLock::new(Default::default())));
        let db_snapshot = CacheDb::new(0, manager);
        NativeDB::with_db_snapshot(db_snapshot).unwrap()
    }

    #[test]
    fn get_after_set() {
        let db = setup_db();

        let key = b"foo".to_vec();
        let value = b"bar".to_vec();
        db.set_values(vec![(key.clone(), Some(value.clone()))], 0)
            .unwrap();
        assert_eq!(db.get_value_option(&key, 0).unwrap(), Some(value.clone()));
        let value2 = b"bar2".to_vec();
        db.set_values(vec![(key.clone(), Some(value2.clone()))], 1)
            .unwrap();
        assert_eq!(db.get_value_option(&key, 0).unwrap(), Some(value));
    }

    #[test]
    fn get_after_delete() {
        let db = setup_db();

        let key = b"deleted".to_vec();
        db.set_values(vec![(key.clone(), None)], 0).unwrap();
        assert_eq!(db.get_value_option(&key, 0).unwrap(), None);
    }

    #[test]
    fn get_nonexistent() {
        let db = setup_db();

        let key = b"spam".to_vec();
        assert_eq!(db.get_value_option(&key, 0).unwrap(), None);
    }
}
