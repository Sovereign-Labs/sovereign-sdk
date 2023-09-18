use std::path::Path;
use std::sync::{Arc, Mutex};

use jmt::storage::{TreeReader, TreeWriter};
use jmt::{KeyHash, Version};
use sov_schema_db::DB;

use crate::rocks_db_config::gen_rocksdb_options;
use crate::schema::tables::{JmtNodes, JmtValues, KeyHashToKey, STATE_TABLES};
use crate::schema::types::StateKey;

/// A typed wrapper around the db for storing rollup state. Internally,
/// this is roughly just an `Arc<SchemaDB>`.
///
/// StateDB implements several convenience functions for state storage -
/// notably the `TreeReader` and `TreeWriter` traits.
#[derive(Clone, Debug)]
pub struct StateDB {
    /// The underlying database instance, wrapped in an [`Arc`] for convenience and [`SchemaDB`] for type safety
    db: Arc<DB>,
    /// The [`Version`] that will be used for the next batch of writes to the DB.
    next_version: Arc<Mutex<Version>>,
}

const STATE_DB_PATH_SUFFIX: &str = "state";

impl StateDB {
    /// Open a [`StateDB`] (backed by RocksDB) at the specified path.
    /// The returned instance will be at the path `{path}/state-db`.
    pub fn with_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let path = path.as_ref().join(STATE_DB_PATH_SUFFIX);
        let inner = DB::open(
            path,
            "state-db",
            STATE_TABLES.iter().copied(),
            &gen_rocksdb_options(&Default::default(), false),
        )?;

        let next_version = Self::last_version_written(&inner)?.unwrap_or_default() + 1;

        Ok(Self {
            db: Arc::new(inner),
            next_version: Arc::new(Mutex::new(next_version)),
        })
    }

    /// Put the preimage of a hashed key into the database. Note that the preimage is not checked for correctness,
    /// since the DB is unaware of the hash function used by the JMT.
    pub fn put_preimage(&self, key_hash: KeyHash, key: &Vec<u8>) -> Result<(), anyhow::Error> {
        self.db.put::<KeyHashToKey>(&key_hash.0, key)
    }

    /// Get an optional value from the database, given a version and a key hash.
    pub fn get_value_option_by_key(
        &self,
        version: Version,
        key: &StateKey,
    ) -> anyhow::Result<Option<jmt::OwnedValue>> {
        let mut iter = self.db.iter::<JmtValues>()?;
        // find the latest instance of the key whose version <= target
        iter.seek_for_prev(&(&key, version))?;
        let found = iter.next();
        match found {
            Some(result) => {
                let ((found_key, found_version), value) = result?;
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

    /// Store an item in the database, given a key, a key hash, a version, and a value
    pub fn update_db(
        &self,
        key: StateKey,
        key_hash: KeyHash,
        value: Option<Vec<u8>>,
        next_version: Version,
    ) -> anyhow::Result<()> {
        self.put_preimage(key_hash, &key)?;
        self.db.put::<JmtValues>(&(key, next_version), &value)?;
        Ok(())
    }

    /// Increment the `next_version` counter by 1.
    pub fn inc_next_version(&self) {
        let mut version = self.next_version.lock().unwrap();
        *version += 1;
    }

    /// Get the current value of the `next_version` counter
    pub fn get_next_version(&self) -> Version {
        let version = self.next_version.lock().unwrap();
        *version
    }

    fn last_version_written(db: &DB) -> anyhow::Result<Option<Version>> {
        let mut iter = db.iter::<JmtValues>()?;
        iter.seek_to_last();

        let version = match iter.next() {
            Some(Ok(((_, version), _))) => Some(version),
            _ => None,
        };
        Ok(version)
    }
}

impl TreeReader for StateDB {
    fn get_node_option(
        &self,
        node_key: &jmt::storage::NodeKey,
    ) -> anyhow::Result<Option<jmt::storage::Node>> {
        self.db.get::<JmtNodes>(node_key)
    }

    fn get_value_option(
        &self,
        version: Version,
        key_hash: KeyHash,
    ) -> anyhow::Result<Option<jmt::OwnedValue>> {
        if let Some(key) = self.db.get::<KeyHashToKey>(&key_hash.0)? {
            self.get_value_option_by_key(version, &key)
        } else {
            Ok(None)
        }
    }

    fn get_rightmost_leaf(
        &self,
    ) -> anyhow::Result<Option<(jmt::storage::NodeKey, jmt::storage::LeafNode)>> {
        todo!()
    }
}

impl TreeWriter for StateDB {
    fn write_node_batch(&self, node_batch: &jmt::storage::NodeBatch) -> anyhow::Result<()> {
        for (node_key, node) in node_batch.nodes() {
            self.db.put::<JmtNodes>(node_key, node)?;
        }

        for ((version, key_hash), value) in node_batch.values() {
            let key_preimage =
                self.db
                    .get::<KeyHashToKey>(&key_hash.0)?
                    .ok_or(anyhow::format_err!(
                        "Could not find preimage for key hash {key_hash:?}"
                    ))?;
            self.db.put::<JmtValues>(&(key_preimage, *version), value)?;
        }
        Ok(())
    }
}

#[cfg(feature = "arbitrary")]
pub mod arbitrary {
    //! Arbitrary definitions for the [`StateDB`].

    use core::ops::{Deref, DerefMut};

    use proptest::strategy::LazyJust;
    use tempfile::TempDir;

    use super::*;

    /// Arbitrary instance of [`StateDB`].
    ///
    /// This is a db wrapper bound to its temporary path that will be deleted once the type is
    /// dropped.
    #[derive(Debug)]
    pub struct ArbitraryDB {
        /// The underlying RocksDB instance.
        pub db: StateDB,
        /// The temporary directory used to create the [`StateDB`].
        pub path: TempDir,
    }

    /// A fallible, arbitrary instance of [`StateDB`].
    ///
    /// This type is suitable for operations that are expected to be infallible. The internal
    /// implementation of the db requires I/O to create the temporary dir, making it fallible.
    #[derive(Debug)]
    pub struct FallibleArbitraryStateDB {
        /// The result of the new db instance.
        pub result: anyhow::Result<ArbitraryDB>,
    }

    impl Deref for ArbitraryDB {
        type Target = StateDB;

        fn deref(&self) -> &Self::Target {
            &self.db
        }
    }

    impl DerefMut for ArbitraryDB {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.db
        }
    }

    impl<'a> ::arbitrary::Arbitrary<'a> for ArbitraryDB {
        fn arbitrary(_u: &mut ::arbitrary::Unstructured<'a>) -> ::arbitrary::Result<Self> {
            let path = TempDir::new().map_err(|_| ::arbitrary::Error::NotEnoughData)?;
            let db = StateDB::with_path(&path).map_err(|_| ::arbitrary::Error::IncorrectFormat)?;
            Ok(Self { db, path })
        }
    }

    impl proptest::arbitrary::Arbitrary for FallibleArbitraryStateDB {
        type Parameters = ();
        type Strategy = LazyJust<Self, fn() -> FallibleArbitraryStateDB>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            fn gen() -> FallibleArbitraryStateDB {
                FallibleArbitraryStateDB {
                    result: TempDir::new()
                        .map_err(|e| {
                            anyhow::anyhow!(format!("failed to generate path for StateDB: {e}"))
                        })
                        .and_then(|path| {
                            let db = StateDB::with_path(&path)?;
                            Ok(ArbitraryDB { db, path })
                        }),
                }
            }
            LazyJust::new(gen)
        }
    }
}

#[cfg(test)]
mod state_db_tests {
    use jmt::storage::{NodeBatch, TreeReader, TreeWriter};
    use jmt::KeyHash;

    use super::StateDB;

    #[test]
    fn test_simple() {
        let tmpdir = tempfile::tempdir().unwrap();
        let db = StateDB::with_path(tmpdir.path()).unwrap();
        let key_hash = KeyHash([1u8; 32]);
        let key = vec![2u8; 100];
        let value = [8u8; 150];

        db.put_preimage(key_hash, &key).unwrap();
        let mut batch = NodeBatch::default();
        batch.extend(vec![], vec![((0, key_hash), Some(value.to_vec()))]);
        db.write_node_batch(&batch).unwrap();

        let found = db.get_value(0, key_hash).unwrap();
        assert_eq!(found, value);

        let found = db.get_value_option_by_key(0, &key).unwrap().unwrap();
        assert_eq!(found, value);
    }
}
