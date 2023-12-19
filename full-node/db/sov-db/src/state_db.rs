use std::path::Path;
use std::sync::{Arc, Mutex};

use jmt::storage::{HasPreimage, TreeReader, TreeWriter};
use jmt::{KeyHash, Version};
use sov_schema_db::snapshot::{DbSnapshot, QueryManager, ReadOnlyDbSnapshot};
use sov_schema_db::SchemaBatch;

use crate::rocks_db_config::gen_rocksdb_options;
use crate::schema::tables::{JmtNodes, JmtValues, KeyHashToKey, STATE_TABLES};
use crate::schema::types::StateKey;

/// A typed wrapper around the db for storing rollup state. Internally,
/// this is roughly just an [`Arc<sov_schema_db::DB>`] with pointer to list of non-finalized snapshots
///
/// StateDB implements several convenience functions for state storage -
/// notably the [`TreeReader`] and [`TreeWriter`] traits.
#[derive(Debug)]
pub struct StateDB<Q> {
    /// The underlying [`DbSnapshot`] that plays as local cache and pointer to previous snapshots and/or [`sov_schema_db::DB`]
    db: Arc<DbSnapshot<Q>>,
    /// The [`Version`] that will be used for the next batch of writes to the DB
    /// This [`Version`] is also used for querying data,
    /// so if this instance of StateDB is used as read only, it won't see newer data.
    next_version: Arc<Mutex<Version>>,
}

// Manual implementation of [`Clone`] to satisfy compiler
impl<Q> Clone for StateDB<Q> {
    fn clone(&self) -> Self {
        StateDB {
            db: self.db.clone(),
            next_version: self.next_version.clone(),
        }
    }
}

impl<Q> StateDB<Q> {
    const DB_PATH_SUFFIX: &'static str = "state";
    const DB_NAME: &'static str = "state-db";

    /// Initialize [`sov_schema_db::DB`] that should be used by snapshots.
    pub fn setup_schema_db(path: impl AsRef<Path>) -> anyhow::Result<sov_schema_db::DB> {
        let state_db_path = path.as_ref().join(Self::DB_PATH_SUFFIX);
        sov_schema_db::DB::open(
            state_db_path,
            Self::DB_NAME,
            STATE_TABLES.iter().copied(),
            &gen_rocksdb_options(&Default::default(), false),
        )
    }

    /// Convert it to [`ReadOnlyDbSnapshot`] which cannot be edited anymore
    pub fn freeze(self) -> anyhow::Result<ReadOnlyDbSnapshot> {
        let inner = Arc::into_inner(self.db).ok_or(anyhow::anyhow!(
            "StateDB underlying DbSnapshot has more than 1 strong references"
        ))?;
        Ok(ReadOnlyDbSnapshot::from(inner))
    }
}

impl<Q: QueryManager> StateDB<Q> {
    /// Creating instance of [`StateDB`] from [`DbSnapshot`]
    pub fn with_db_snapshot(db_snapshot: DbSnapshot<Q>) -> anyhow::Result<Self> {
        let next_version = Self::next_version_from(&db_snapshot)?;
        Ok(Self {
            db: Arc::new(db_snapshot),
            next_version: Arc::new(Mutex::new(next_version)),
        })
    }

    /// Put the preimage of a hashed key into the database. Note that the preimage is not checked for correctness,
    /// since the DB is unaware of the hash function used by the JMT.
    pub fn put_preimages<'a>(
        &self,
        items: impl IntoIterator<Item = (KeyHash, &'a Vec<u8>)>,
    ) -> Result<(), anyhow::Error> {
        let mut batch = SchemaBatch::new();
        for (key_hash, key) in items.into_iter() {
            batch.put::<KeyHashToKey>(&key_hash.0, key)?;
        }
        self.db.write_many(batch)?;
        Ok(())
    }

    /// Get an optional value from the database, given a version and a key hash.
    pub fn get_value_option_by_key(
        &self,
        version: Version,
        key: &StateKey,
    ) -> anyhow::Result<Option<jmt::OwnedValue>> {
        let found = self.db.get_prev::<JmtValues>(&(&key, version))?;
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

    /// Used to always query for latest possible version!
    pub fn max_out_next_version(&self) {
        let mut version = self.next_version.lock().unwrap();
        *version = u64::MAX - 1;
    }

    fn next_version_from(db_snapshot: &DbSnapshot<Q>) -> anyhow::Result<Version> {
        let last_key_value = db_snapshot.get_largest::<JmtNodes>()?;
        let largest_version = last_key_value.map(|(k, _)| k.version());
        let next_version = largest_version
            .unwrap_or_default()
            .checked_add(1)
            .expect("JMT Version overflow. Is is over");
        Ok(next_version)
    }
}

impl<Q: QueryManager> TreeReader for StateDB<Q> {
    fn get_node_option(
        &self,
        node_key: &jmt::storage::NodeKey,
    ) -> anyhow::Result<Option<jmt::storage::Node>> {
        self.db.read::<JmtNodes>(node_key)
    }

    fn get_value_option(
        &self,
        version: Version,
        key_hash: KeyHash,
    ) -> anyhow::Result<Option<jmt::OwnedValue>> {
        if let Some(key) = self.db.read::<KeyHashToKey>(&key_hash.0)? {
            self.get_value_option_by_key(version, &key)
        } else {
            Ok(None)
        }
    }

    fn get_rightmost_leaf(
        &self,
    ) -> anyhow::Result<Option<(jmt::storage::NodeKey, jmt::storage::LeafNode)>> {
        todo!("StateDB does not support [`TreeReader::get_rightmost_leaf`] yet")
    }
}

impl<Q: QueryManager> TreeWriter for StateDB<Q> {
    fn write_node_batch(&self, node_batch: &jmt::storage::NodeBatch) -> anyhow::Result<()> {
        let mut batch = SchemaBatch::new();
        for (node_key, node) in node_batch.nodes() {
            batch.put::<JmtNodes>(node_key, node)?;
        }

        for ((version, key_hash), value) in node_batch.values() {
            let key_preimage =
                self.db
                    .read::<KeyHashToKey>(&key_hash.0)?
                    .ok_or(anyhow::format_err!(
                        "Could not find preimage for key hash {key_hash:?}. Has `StateDB::put_preimage` been called for this key?"
                    ))?;
            batch.put::<JmtValues>(&(key_preimage, *version), value)?;
        }
        self.db.write_many(batch)?;
        Ok(())
    }
}

impl<Q: QueryManager> HasPreimage for StateDB<Q> {
    fn preimage(&self, key_hash: KeyHash) -> anyhow::Result<Option<Vec<u8>>> {
        self.db.read::<KeyHashToKey>(&key_hash.0)
    }
}

#[cfg(test)]
mod state_db_tests {
    use std::sync::{Arc, RwLock};

    use jmt::storage::{NodeBatch, TreeReader, TreeWriter};
    use jmt::KeyHash;
    use sov_schema_db::snapshot::{DbSnapshot, NoopQueryManager, ReadOnlyLock};

    use super::StateDB;

    #[test]
    fn test_simple() {
        let manager = ReadOnlyLock::new(Arc::new(RwLock::new(Default::default())));
        let db_snapshot = DbSnapshot::<NoopQueryManager>::new(0, manager);
        let db = StateDB::with_db_snapshot(db_snapshot).unwrap();
        let key_hash = KeyHash([1u8; 32]);
        let key = vec![2u8; 100];
        let value = [8u8; 150];

        db.put_preimages(vec![(key_hash, &key)]).unwrap();
        let mut batch = NodeBatch::default();
        batch.extend(vec![], vec![((0, key_hash), Some(value.to_vec()))]);
        db.write_node_batch(&batch).unwrap();

        let found = db.get_value(0, key_hash).unwrap();
        assert_eq!(found, value);

        let found = db.get_value_option_by_key(0, &key).unwrap().unwrap();
        assert_eq!(found, value);
    }
}
