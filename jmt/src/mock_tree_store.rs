// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use crate::{
    hash::TreeHash,
    node_type::{LeafNode, Node, NodeKey, PhysicalNode, PhysicalNodeKey},
    test_utils::TestKey,
    Key, NodeBatch, PhysicalTreeReader, PhysicalTreeWriter, StaleNodeIndex, TreeReader,
    TreeUpdateBatch, TreeWriter, TypedStore, Version,
};
use std::{
    collections::{hash_map::Entry, BTreeSet, HashMap},
    error::Error,
    fmt::Debug,
    sync::RwLock,
};

pub struct MemTreeStore<K> {
    data: RwLock<HashMap<PhysicalNodeKey, PhysicalNode<K>>>,
}

pub struct MockTreeStore<K, H, const N: usize> {
    /// A redundant TreeReader implementation to test out the TypedStore struct
    wrapped_physical_store: TypedStore<MemTreeStore<K>, H, N>,
    /// The primary backing store for MockTreeStore
    data: RwLock<(
        HashMap<NodeKey<N>, Node<K, H, N>>,
        BTreeSet<StaleNodeIndex<N>>,
    )>,
    allow_overwrite: bool,
}

impl<K: Clone> PhysicalTreeReader<K> for MemTreeStore<K> {
    type Error = anyhow::Error;

    fn get_physical_node(
        &self,
        node_key: &PhysicalNodeKey,
    ) -> std::result::Result<PhysicalNode<K>, Self::Error> {
        self.get_physical_node_option(node_key)?
            .ok_or_else(|| TestTreeError::MissingNode.into())
    }

    fn get_physical_node_option(
        &self,
        node_key: &PhysicalNodeKey,
    ) -> std::result::Result<Option<PhysicalNode<K>>, Self::Error> {
        Ok(self
            .data
            .read()
            .expect("Lock must not be poisoned")
            .get(node_key)
            .cloned())
    }

    fn get_value(&self, _key: &(Version, K)) -> std::result::Result<Option<Vec<u8>>, Self::Error> {
        todo!()
    }

    fn get_rightmost_physical_leaf(
        &self,
        _version: Version,
    ) -> std::result::Result<
        Option<(PhysicalNodeKey, crate::node_type::PhysicalLeafNode<K>)>,
        Self::Error,
    > {
        todo!()
    }
}

impl<K: Clone + Send + Sync> PhysicalTreeWriter<K> for MemTreeStore<K> {
    type Error = anyhow::Error;

    fn write_physical_node_batch(
        &self,
        node_batch: &crate::PhysicalNodeBatch<K>,
    ) -> std::result::Result<(), Self::Error> {
        let mut store = self.data.write().unwrap();
        for (node_key, node) in node_batch.clone() {
            store.insert(node_key, node);
        }
        Ok(())
    }
}

impl<K, H, const N: usize> Default for MockTreeStore<K, H, N> {
    fn default() -> Self {
        Self {
            wrapped_physical_store: TypedStore::new(MemTreeStore {
                data: Default::default(),
            }),
            data: RwLock::new((HashMap::new(), BTreeSet::new())),
            allow_overwrite: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct BoxError(#[from] Box<dyn Error + Send + Sync + 'static>);

type Result<T, E = BoxError> = core::result::Result<T, E>;
impl<K, H, const N: usize> TreeReader<K, H, N> for MockTreeStore<K, H, N>
where
    K: crate::test_utils::TestKey,
    H: TreeHash<N>,
{
    type Error = anyhow::Error;
    fn get_node_option(&self, node_key: &NodeKey<N>) -> Result<Option<Node<K, H, N>>, Self::Error> {
        // For every query, fetch the node from both backing stores and ensure that they agree
        let raw_node = self.data.read().unwrap().0.get(node_key).cloned();
        let transformed_node = self.wrapped_physical_store.get_node_option(node_key)?;
        assert_eq!(&raw_node, &transformed_node);

        Ok(raw_node)
    }

    fn get_value(&self, _key: &(Version, K)) -> std::result::Result<Option<Vec<u8>>, Self::Error> {
        unimplemented!()
    }

    fn get_rightmost_leaf(
        &self,
        version: Version,
    ) -> Result<Option<(NodeKey<N>, LeafNode<K, H, N>)>, Self::Error> {
        let locked = self.data.read().unwrap();
        let mut node_key_and_node: Option<(NodeKey<N>, LeafNode<K, H, N>)> = None;

        for (key, value) in locked.0.iter() {
            if let Node::Leaf(leaf_node) = value {
                if key.version() == version
                    && (node_key_and_node.is_none()
                        || leaf_node.account_key()
                            > node_key_and_node.as_ref().unwrap().1.account_key())
                {
                    node_key_and_node.replace((key.clone(), leaf_node.clone()));
                }
            }
        }

        Ok(node_key_and_node)
    }

    fn get_node(&self, node_key: &NodeKey<N>) -> std::result::Result<Node<K, H, N>, Self::Error> {
        self.get_node_option(node_key)?
            .ok_or_else(|| TestTreeError::MissingNode.into())
    }
}

impl<K: Key, H: TreeHash<N>, const N: usize> TreeWriter<K, H, N> for MockTreeStore<K, H, N>
where
    K: TestKey,
{
    fn write_node_batch(&self, node_batch: &NodeBatch<K, H, N>) -> Result<(), BoxError> {
        // Store each item in the primary store
        let mut locked = self.data.write().unwrap();
        for (node_key, node) in node_batch.clone() {
            let replaced = locked.0.insert(node_key, node);
            if !self.allow_overwrite {
                assert_eq!(replaced, None);
            }
        }

        self.wrapped_physical_store
            .write_node_batch(node_batch)
            .expect("insertion to btree must succeed");
        Ok(())
    }

    type Error = BoxError;
}

#[derive(Debug, thiserror::Error)]
enum TestTreeError {
    #[error("Key {0:} exists")]
    ErrKeyExists(String),
    #[error("Duplicated retire log.")]
    ErrDuplicatedRetireLog,
    #[error("Stale node index refers to non-existent node.")]
    StaleNodeIndexDne,
    #[error("Node does not exist")]
    MissingNode,
}

impl<K, H, const N: usize> MockTreeStore<K, H, N>
where
    K: TestKey,
{
    pub fn new(allow_overwrite: bool) -> Self {
        Self {
            allow_overwrite,
            ..Default::default()
        }
    }

    pub fn put_node(&self, node_key: NodeKey<N>, node: Node<K, H, N>) -> Result<()> {
        match self.data.write().unwrap().0.entry(node_key.clone()) {
            Entry::Occupied(o) => {
                return Err(BoxError(Box::new(TestTreeError::ErrKeyExists(format!(
                    "{:?}",
                    &o.key()
                )))));
            }
            Entry::Vacant(v) => {
                v.insert(node.clone());
            }
        }

        self.wrapped_physical_store
            .inner
            .data
            .write()
            .unwrap()
            .insert(node_key.into(), node.into());
        Ok(())
    }

    fn put_stale_node_index(&self, index: StaleNodeIndex<N>) -> Result<()> {
        let is_new_entry = self.data.write().unwrap().1.insert(index);
        if !is_new_entry {
            return Err(BoxError(Box::new(TestTreeError::ErrDuplicatedRetireLog)));
        }
        Ok(())
    }

    pub fn write_tree_update_batch(&self, batch: TreeUpdateBatch<K, H, N>) -> Result<()> {
        batch
            .node_batch
            .into_iter()
            .flatten()
            .map(|(k, v)| self.put_node(k, v))
            .collect::<Result<Vec<_>>>()?;
        batch
            .stale_node_index_batch
            .into_iter()
            .flatten()
            .map(|i| self.put_stale_node_index(i))
            .collect::<Result<Vec<_>>>()?;
        Ok(())
    }

    pub fn purge_stale_nodes(&self, min_readable_version: Version) -> Result<()> {
        let mut wlocked = self.data.write().unwrap();

        // Only records retired before or at `min_readable_version` can be purged in order
        // to keep that version still readable.
        let to_prune = wlocked
            .1
            .iter()
            .take_while(|log| log.stale_since_version <= min_readable_version)
            .cloned()
            .collect::<Vec<_>>();

        for log in to_prune {
            let removed = wlocked.0.remove(&log.node_key).is_some();
            if !removed {
                return Err(BoxError(Box::new(TestTreeError::StaleNodeIndexDne)));
            }
            wlocked.1.remove(&log);
        }

        Ok(())
    }

    pub fn num_nodes(&self) -> usize {
        self.data.read().unwrap().0.len()
    }
}
