use std::path::Path;
use std::sync::Arc;

use sov_schema_db::DB;

use crate::rocks_db_config::gen_rocksdb_options;
use crate::schema::tables::{ModuleAccessoryState, NATIVE_TABLES};
use crate::schema::types::{DbBytes, StateKey};

/// A typed wrapper around RocksDB for storing native-only accessory state.
/// Internally, this is roughly just an [`Arc<SchemaDB>`].
#[derive(Clone)]
pub struct NativeDB {
    /// The underlying RocksDB instance, wrapped in an [`Arc`] for convenience
    /// and [`DB`] for type safety.
    db: Arc<DB>,
}

impl NativeDB {
    const DB_PATH_SUFFIX: &str = "native";
    const DB_NAME: &str = "native-db";

    /// Opens a [`NativeDB`] (backed by RocksDB) at the specified path.
    /// The returned instance will be at the path `{path}/native-db`.
    pub fn with_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let path = path.as_ref().join(Self::DB_PATH_SUFFIX);
        let inner = DB::open(
            path,
            Self::DB_NAME,
            NATIVE_TABLES.iter().copied(),
            &gen_rocksdb_options(&Default::default(), false),
        )?;

        Ok(Self {
            db: Arc::new(inner),
        })
    }

    /// Queries for a value in the [`NativeDB`], given a key.
    pub fn get_value_option(&self, key: &StateKey) -> anyhow::Result<Option<DbBytes>> {
        let value = self.db.get::<ModuleAccessoryState>(key)?;
        match value {
            Some(value) if value.as_ref().is_empty() => Ok(None),
            Some(value) => Ok(Some(value)),
            None => Ok(None),
        }
    }

    /// Sets a key-value pair in the [`NativeDB`].
    pub fn set_value(&self, key: Vec<u8>, value: Option<Vec<u8>>) -> anyhow::Result<()> {
        self.db
            .put::<ModuleAccessoryState>(&key, &DbBytes::new(value.unwrap_or_default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_after_set() {
        let tmpdir = tempfile::tempdir().unwrap();
        let db = NativeDB::with_path(tmpdir.path()).unwrap();

        let key = b"foo".to_vec();
        let value = b"bar".to_vec();
        db.set_value(key.clone(), Some(value.clone())).unwrap();
        assert_eq!(
            db.get_value_option(&key).unwrap(),
            Some(DbBytes::new(value))
        );
    }

    #[test]
    fn get_after_delete() {
        let tmpdir = tempfile::tempdir().unwrap();
        let db = NativeDB::with_path(tmpdir.path()).unwrap();

        let key = b"deleted".to_vec();
        db.set_value(key.clone(), None).unwrap();
        assert_eq!(db.get_value_option(&key).unwrap(), None);
    }

    #[test]
    fn get_nonexistent() {
        let tmpdir = tempfile::tempdir().unwrap();
        let db = NativeDB::with_path(tmpdir.path()).unwrap();

        let key = b"spam".to_vec();
        assert_eq!(db.get_value_option(&key).unwrap(), None);
    }
}
