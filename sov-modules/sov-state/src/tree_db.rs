use std::{cell::RefCell, path::Path};

use jmt::{
    storage::{Node, TreeReader, TreeWriter},
    KeyHash, OwnedValue, Version,
};
use sovereign_db::state_db::StateDB;

use crate::{
    storage::{StorageKey, StorageValue},
    ValueReader,
};

#[derive(Clone)]
pub struct TreeReadLogger {
    is_recording: bool,
    nodes: RefCell<Vec<Option<Node>>>,
    values: RefCell<Vec<Option<OwnedValue>>>,
    state_db: StateDB,
}

impl ValueReader for TreeReadLogger {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue> {
        self.state_db.read_value(key)
    }
}

impl Into<ZkTreeDb> for TreeReadLogger {
    fn into(self) -> ZkTreeDb {
        ZkTreeDb {
            nodes: RefCell::new(self.nodes.take().into_iter()),
            values: RefCell::new(self.values.take().into_iter()),
            next_version: self.get_next_version(),
        }
    }
}

impl TreeReadLogger {
    /// Creates a tree read logger wrapping the provided StateDB.
    /// The logger is recording by default
    pub fn with_db(db: StateDB) -> Self {
        Self {
            is_recording: true,
            nodes: Default::default(),
            values: Default::default(),
            state_db: db,
        }
    }

    /// Opens a StateDB at the provided path, and creates a new logger wrapping it.
    /// The logger is recording by default
    pub fn with_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let db = StateDB::with_path(path)?;
        Ok(Self::with_db(db))
    }

    /// Creates a tree read logger wrapping a temporary StateDB.
    /// The logger is recording by default
    #[cfg(any(test, feature = "temp"))]
    pub fn temporary() -> Self {
        let db = StateDB::temporary();
        Self::with_db(db)
    }

    /// Causes the tree read logger to start recording, if it wasn't already.
    /// This incurs one `clone` of any item read from the StateDB using the
    /// `TreeReader` trait.
    #[allow(unused)]
    pub fn start_recording(&mut self) {
        self.is_recording = true
    }

    /// Causes the tree read logger to stop recording if it was previously running.
    /// Disabling recording. A logger that is not recording adds no overhead per-read except for a single comparison.
    #[allow(unused)]
    pub fn stop_recording(&mut self) {
        self.is_recording = false
    }

    pub fn put_preimage(&self, key_hash: KeyHash, key: &Vec<u8>) -> Result<(), anyhow::Error> {
        self.state_db.put_preimage(key_hash, key)
    }

    pub fn get_next_version(&self) -> Version {
        self.state_db.get_next_version()
    }

    pub fn inc_next_version(&self) {
        self.state_db.inc_next_version()
    }
}

impl TreeReader for TreeReadLogger {
    fn get_node_option(
        &self,
        node_key: &jmt::storage::NodeKey,
    ) -> anyhow::Result<Option<jmt::storage::Node>> {
        let node_opt = self.state_db.get_node_option(node_key)?;
        if self.is_recording {
            self.nodes.borrow_mut().push(node_opt.clone())
        }
        Ok(node_opt)
    }

    fn get_value_option(
        &self,
        max_version: jmt::Version,
        key_hash: jmt::KeyHash,
    ) -> anyhow::Result<Option<OwnedValue>> {
        let value_opt = self.state_db.get_value_option(max_version, key_hash)?;
        if self.is_recording {
            self.values.borrow_mut().push(value_opt.clone())
        }
        Ok(value_opt)
    }

    fn get_rightmost_leaf(
        &self,
    ) -> anyhow::Result<Option<(jmt::storage::NodeKey, jmt::storage::LeafNode)>> {
        self.state_db.get_rightmost_leaf()
    }
}

impl TreeWriter for TreeReadLogger {
    fn write_node_batch(&self, node_batch: &jmt::storage::NodeBatch) -> anyhow::Result<()> {
        self.state_db.write_node_batch(node_batch)
    }
}

/// A ZkTreeDb is just a log of the values read by another TreeReader while executing
/// a particular sequence of jmt operations. It can can be used to emulate a TreeReader
/// when performing the same sequence of operations in the zk context.
pub struct ZkTreeDb {
    nodes: RefCell<std::vec::IntoIter<Option<Node>>>,
    values: RefCell<std::vec::IntoIter<Option<OwnedValue>>>,
    pub next_version: Version,
}

impl ZkTreeDb {
    #[cfg(test)]
    pub fn empty() -> Self {
        ZkTreeDb {
            nodes: RefCell::new(vec![].into_iter()),
            values: RefCell::new(vec![].into_iter()),
            next_version: 0,
        }
    }
}

impl TreeReader for ZkTreeDb {
    fn get_node_option(
        &self,
        _node_key: &jmt::storage::NodeKey,
    ) -> anyhow::Result<Option<jmt::storage::Node>> {
        Ok(self
            .nodes
            .borrow_mut()
            .next()
            .expect("Read must have been recorded"))
    }

    fn get_value_option(
        &self,
        _max_version: jmt::Version,
        _key_hash: jmt::KeyHash,
    ) -> anyhow::Result<Option<jmt::OwnedValue>> {
        Ok(self
            .values
            .borrow_mut()
            .next()
            .expect("Read must have been recorded"))
    }

    fn get_rightmost_leaf(
        &self,
    ) -> anyhow::Result<Option<(jmt::storage::NodeKey, jmt::storage::LeafNode)>> {
        unimplemented!()
    }
}
