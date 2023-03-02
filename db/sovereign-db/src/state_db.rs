use std::{path::Path, sync::Arc};

use jmt::{
    storage::{TreeReader, TreeWriter},
    KeyHash, Version,
};
use schemadb::DB;

use crate::{
    rocks_db_config::gen_rocksdb_options,
    schema::{
        tables::{JmtNodes, JmtValues, KeyHashToKey, STATE_TABLES},
        types::StateKey,
    },
};

#[derive(Clone)]
pub struct StateDB {
    db: Arc<DB>,
}

impl StateDB {
    pub fn with_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let inner = DB::open(
            path,
            "state-db",
            STATE_TABLES.iter().map(|x| *x),
            &gen_rocksdb_options(&Default::default(), false),
        )?;
        Ok(Self {
            db: Arc::new(inner),
        })
    }

    /// A rocksdb instance which stores its data in a tempdir
    #[cfg(any(test, feature = "temp"))]
    pub fn temporary() -> Self {
        let path = schemadb::temppath::TempPath::new();
        Self::with_path(path).unwrap()
    }

    pub fn put_preimage(&self, key_hash: KeyHash, key: &Vec<u8>) -> Result<(), anyhow::Error> {
        self.db.put::<KeyHashToKey>(&key_hash.0, key)
    }

    pub fn get_value_option_by_key(
        &self,
        version: Version,
        key: StateKey,
    ) -> anyhow::Result<Option<jmt::OwnedValue>> {
        let mut iter = self.db.iter::<JmtValues>()?;
        // find the latest instance of the key whose version <= target
        iter.seek_for_prev(&(&key, version))?;
        let found = iter.next();
        match found {
            Some(result) => {
                let ((found_key, found_version), value) = result?;
                if found_key == key {
                    anyhow::ensure!(found_version <= version, "Bug! iterator isn't returning expected values. expected a version <= {version:} but found {found_version:}");
                    Ok(value)
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
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
            self.get_value_option_by_key(version, key)
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

#[cfg(test)]
mod state_db_tests {
    use jmt::{
        storage::{NodeBatch, TreeReader, TreeWriter},
        KeyHash,
    };

    use super::StateDB;

    #[test]
    fn test_simple() {
        let db = StateDB::temporary();
        let key_hash = KeyHash([1u8; 32]);
        let key = vec![2u8; 100];
        let value = [8u8; 150];

        db.put_preimage(key_hash, &key).unwrap();
        let mut batch = NodeBatch::default();
        batch.extend(vec![], vec![((0, key_hash), Some(value.to_vec()))]);
        db.write_node_batch(&batch).unwrap();

        let found = db.get_value(0, key_hash).unwrap();
        assert_eq!(found, value)
    }
}
