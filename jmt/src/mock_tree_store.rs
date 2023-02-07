// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use crate::{
    hash::TreeHash,
    node_type::{LeafNode, Node, NodeKey},
    test_utils::TestKey,
    Key, NodeBatch, StaleNodeIndex, TreeReader, TreeUpdateBatch, TreeWriter, Version,
};
use std::{
    collections::{hash_map::Entry, BTreeSet, HashMap},
    error::Error,
    fmt::Debug,
    sync::RwLock,
};

pub struct MockTreeStore<K, H, const N: usize> {
    data: RwLock<(
        HashMap<NodeKey<N>, Node<K, H, N>>,
        BTreeSet<StaleNodeIndex<N>>,
    )>,
    allow_overwrite: bool,
}

impl<K, H, const N: usize> Default for MockTreeStore<K, H, N> {
    fn default() -> Self {
        Self {
            data: RwLock::new((HashMap::new(), BTreeSet::new())),
            allow_overwrite: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct BoxError(#[from] Box<dyn Error + Send + Sync + 'static>);

// #[derive(Debug, thiserror::Error)]
// struct InvalidNullError;

// impl std::fmt::Display for InvalidNullError {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.write_str("invalid null")
//     }
// }

// #[derive(Debug, thiserror::Error)]
// struct MissingNodeError;

// impl std::fmt::Display for MissingNodeError {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.write_str("missing node")
//     }
// }

type Result<T, E = BoxError> = core::result::Result<T, E>;
impl<K, H, const N: usize> TreeReader<K, H, N> for MockTreeStore<K, H, N>
where
    K: crate::test_utils::TestKey,
    H: TreeHash<N>,
{
    type Error = anyhow::Error;
    fn get_node_option(&self, node_key: &NodeKey<N>) -> Result<Option<Node<K, H, N>>, Self::Error> {
        Ok(self.data.read().unwrap().0.get(node_key).cloned())
    }

    fn get_value(&self, key: &(Version, K)) -> std::result::Result<Option<Vec<u8>>, Self::Error> {
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
        let mut locked = self.data.write().unwrap();
        for (node_key, node) in node_batch.clone() {
            let replaced = locked.0.insert(node_key, node);
            if !self.allow_overwrite {
                assert_eq!(replaced, None);
            }
        }
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
        match self.data.write().unwrap().0.entry(node_key) {
            Entry::Occupied(o) => {
                return Err(BoxError(Box::new(TestTreeError::ErrKeyExists(format!(
                    "{:?}",
                    &o.key()
                )))));
            }
            Entry::Vacant(v) => {
                v.insert(node);
            }
        }
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
