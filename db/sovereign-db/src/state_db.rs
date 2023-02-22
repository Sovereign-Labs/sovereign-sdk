use std::sync::Arc;

use jmt::{storage::TreeReader, KeyHash, Version};
use schemadb::DB;

use crate::schema::tables::{JmtNodes, JmtValues, KeyHashToKey};

pub struct StateDB {
    db: Arc<DB>,
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
            return Ok(self
                .db
                .get::<JmtValues>(&(version, key))?
                .map(|v| v.as_ref().to_vec()));
        }
        Ok(None)
    }

    fn get_rightmost_leaf(
        &self,
    ) -> anyhow::Result<Option<(jmt::storage::NodeKey, jmt::storage::LeafNode)>> {
        todo!()
    }
}
