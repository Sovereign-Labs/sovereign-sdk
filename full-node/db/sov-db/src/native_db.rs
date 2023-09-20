use std::path::Path;
use std::sync::Arc;

use sov_schema_db::{SchemaBatch, DB};

use crate::rocks_db_config::gen_rocksdb_options;
use crate::schema::tables::{ModuleAccessoryState, NATIVE_TABLES};
use crate::schema::types::StateKey;

/// A typed wrapper around RocksDB for storing native-only accessory state.
/// Internally, this is roughly just an [`Arc<SchemaDB>`].
#[derive(Clone, Debug)]
pub struct NativeDB {
    /// The underlying RocksDB instance, wrapped in an [`Arc`] for convenience
    /// and [`DB`] for type safety.
    db: Arc<DB>,
}

impl NativeDB {
    const DB_PATH_SUFFIX: &'static str = "native";
    const DB_NAME: &'static str = "native-db";

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
    pub fn get_value_option(&self, key: &StateKey) -> anyhow::Result<Option<Vec<u8>>> {
        self.db
            .get::<ModuleAccessoryState>(key)
            .map(Option::flatten)
    }

    /// Sets a key-value pair in the [`NativeDB`].
    pub fn set_value(&self, key: Vec<u8>, value: Option<Vec<u8>>) -> anyhow::Result<()> {
        self.set_values(vec![(key, value)])
    }

    /// Sets a sequence of key-value pairs in the [`NativeDB`]. The write is atomic.
    pub fn set_values(
        &self,
        key_value_pairs: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    ) -> anyhow::Result<()> {
        let batch = SchemaBatch::default();
        for (key, value) in key_value_pairs {
            batch.put::<ModuleAccessoryState>(&key, &value)?;
        }
        self.db.write_schemas(batch)
    }
}

#[cfg(feature = "arbitrary")]
pub mod arbitrary {
    //! Arbitrary definitions for the [`NativeDB`].

    use core::ops::{Deref, DerefMut};

    use proptest::strategy::LazyJust;
    use tempfile::TempDir;

    use super::*;

    /// Arbitrary instance of [`NativeDB`].
    ///
    /// This is a db wrapper bound to its temporary path that will be deleted once the type is
    /// dropped.
    #[derive(Debug)]
    pub struct ArbitraryNativeDB {
        /// The underlying RocksDB instance.
        pub db: NativeDB,
        /// The temporary directory used to create the [`NativeDB`].
        pub path: TempDir,
    }

    /// A fallible, arbitrary instance of [`NativeDB`].
    ///
    /// This type is suitable for operations that are expected to be infallible. The internal
    /// implementation of the db requires I/O to create the temporary dir, making it fallible.
    #[derive(Debug)]
    pub struct FallibleArbitraryNativeDB {
        /// The result of the new db instance.
        pub result: anyhow::Result<ArbitraryNativeDB>,
    }

    impl Deref for ArbitraryNativeDB {
        type Target = NativeDB;

        fn deref(&self) -> &Self::Target {
            &self.db
        }
    }

    impl DerefMut for ArbitraryNativeDB {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.db
        }
    }

    impl<'a> ::arbitrary::Arbitrary<'a> for ArbitraryNativeDB {
        fn arbitrary(_u: &mut ::arbitrary::Unstructured<'a>) -> ::arbitrary::Result<Self> {
            let path = TempDir::new().map_err(|_| ::arbitrary::Error::NotEnoughData)?;
            let db = NativeDB::with_path(&path).map_err(|_| ::arbitrary::Error::IncorrectFormat)?;
            Ok(Self { db, path })
        }
    }

    impl proptest::arbitrary::Arbitrary for FallibleArbitraryNativeDB {
        type Parameters = ();
        type Strategy = LazyJust<Self, fn() -> FallibleArbitraryNativeDB>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            fn gen() -> FallibleArbitraryNativeDB {
                FallibleArbitraryNativeDB {
                    result: TempDir::new()
                        .map_err(|e| {
                            anyhow::anyhow!(format!("failed to generate path for NativeDB: {e}"))
                        })
                        .and_then(|path| {
                            let db = NativeDB::with_path(&path)?;
                            Ok(ArbitraryNativeDB { db, path })
                        }),
                }
            }
            LazyJust::new(gen)
        }
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
        db.set_values(vec![(key.clone(), Some(value.clone()))])
            .unwrap();
        assert_eq!(db.get_value_option(&key).unwrap(), Some(value));
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
