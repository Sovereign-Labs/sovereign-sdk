// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0
// Adapted from aptos-labs/jellyfish-merkle
// Modified to be generic over choice of hash function

#![forbid(unsafe_code)]

//! This module implements [`JellyfishMerkleTree`] backed by storage module. The tree itself doesn't
//! persist anything, but realizes the logic of R/W only. The write path will produce all the
//! intermediate results in a batch for storage layer to commit and the read path will return
//! results directly. The public APIs are only [`new`], [`batch_put_value_set`], and
//! [`get_with_proof`]. After each put with a `value_set` based on a known version, the tree will
//! return a new root hash with a [`TreeUpdateBatch`] containing all the new nodes and indices of
//! stale nodes.
//!
//! A Jellyfish Merkle Tree itself logically is a 256-bit sparse Merkle tree with an optimization
//! that any subtree containing 0 or 1 leaf node will be replaced by that leaf node or a placeholder
//! node with default hash value. With this optimization we can save CPU by avoiding hashing on
//! many sparse levels in the tree. Physically, the tree is structurally similar to the modified
//! Patricia Merkle tree of Ethereum but with some modifications. A standard Jellyfish Merkle tree
//! will look like the following figure:
//!
//! ```text
//!                                    .──────────────────────.
//!                            _.─────'                        `──────.
//!                       _.──'                                        `───.
//!                   _.─'                                                  `──.
//!               _.─'                                                          `──.
//!             ,'                                                                  `.
//!          ,─'                                                                      '─.
//!        ,'                                                                            `.
//!      ,'                                                                                `.
//!     ╱                                                                                    ╲
//!    ╱                                                                                      ╲
//!   ╱                                                                                        ╲
//!  ╱                                                                                          ╲
//! ;                                                                                            :
//! ;                                                                                            :
//!;                                                                                              :
//!│                                                                                              │
//!+──────────────────────────────────────────────────────────────────────────────────────────────+
//! .''.  .''.  .''.  .''.  .''.  .''.  .''.  .''.  .''.  .''.  .''.  .''.  .''.  .''.  .''.  .''.
//!/    \/    \/    \/    \/    \/    \/    \/    \/    \/    \/    \/    \/    \/    \/    \/    \
//!+----++----++----++----++----++----++----++----++----++----++----++----++----++----++----++----+
//! (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (
//!  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )
//! (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (
//!  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )
//! (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (
//!  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )
//! (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (
//!  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )  )
//! (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (  (
//! ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■  ■
//! ■: the [`Value`] type this tree stores.
//! ```
//!
//! A Jellyfish Merkle Tree consists of [`InternalNode`] and [`LeafNode`]. [`InternalNode`] is like
//! branch node in ethereum patricia merkle with 16 children to represent a 4-level binary tree and
//! [`LeafNode`] is similar to that in patricia merkle too. In the above figure, each `bell` in the
//! jellyfish is an [`InternalNode`] while each tentacle is a [`LeafNode`]. It is noted that
//! Jellyfish merkle doesn't have a counterpart for `extension` node of ethereum patricia merkle.
//!
//! This implementation of the JMT stores only value hashes and not the values themselves. For
//! context on this decision, see [Aptos Core Issue 402](https://github.com/aptos-labs/aptos-core/issues/402)
//!
//! [`JellyfishMerkleTree`]: struct.JellyfishMerkleTree.html
//! [`new`]: struct.JellyfishMerkleTree.html#method.new
//! [`put_value_sets`]: struct.JellyfishMerkleTree.html#method.put_value_sets
//! [`put_value_set`]: struct.JellyfishMerkleTree.html#method.put_value_set
//! [`get_with_proof`]: struct.JellyfishMerkleTree.html#method.get_with_proof
//! [`TreeUpdateBatch`]: struct.TreeUpdateBatch.html
//! [`InternalNode`]: node_type/struct.InternalNode.html
//! [`LeafNode`]: node_type/struct.LeafNode.html

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Debug,
    marker::PhantomData,
};

use errors::CodecError;
use hash::{HashOutput, HashValueBitIterator, TreeHash};
use metrics::{inc_deletion_count_if_enabled, set_leaf_count_if_enabled};
use node_type::{
    Child, Children, InternalNode, LeafNode, Node, NodeKey, PhysicalLeafNode, PhysicalNode,
    PhysicalNodeKey,
};
use parallel::{parallel_process_range_if_enabled, run_on_io_pool_if_enabled};
use proof::{SparseMerkleProof, SparseMerkleProofExt, SparseMerkleRangeProof};
#[cfg(any(test, feature = "fuzzing"))]
use proptest_derive::Arbitrary;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;
use types::nibble::{nibble_path::NibblePath, Nibble};

pub mod errors;
pub mod hash;
#[cfg(any(test))]
pub mod jellyfish_merkle_test;
pub mod metrics;
#[cfg(any(test, feature = "fuzzing"))]
pub mod mock_tree_store;
pub mod node_type;
pub mod parallel;
pub mod proof;
#[cfg(any(test, feature = "fuzzing"))]
pub mod test_helper;
pub mod types;

pub type Version = u64;

#[cfg(any(test, feature = "fuzzing"))]
/// The size of HashValues for testing
pub const TEST_DIGEST_SIZE: usize = 32;

// TODO(preston-evans98): consider removing AsRef<u8> and TryFrom in favor of a concrete
// serde serialization scheme
pub trait Key:
    AsRef<[u8]>
    + for<'a> TryFrom<&'a [u8], Error = Self::FromBytesErr>
    + Clone
    + Serialize
    + DeserializeOwned
    + Send
    + Sync
    + PartialEq
    + 'static
    + Debug
{
    type FromBytesErr: std::error::Error + Sized;
    fn key_size(&self) -> usize;
}

/// `TreeReader` defines the interface between
/// [`JellyfishMerkleTree`](struct.JellyfishMerkleTree.html)
/// and underlying storage holding nodes.
pub trait TreeReader<K, H, const N: usize> {
    type Error: Into<anyhow::Error> + Send + Sync + 'static;
    /// Gets node given a node key. Returns error if the node does not exist.
    ///
    /// Recommended impl:
    /// ```ignore
    /// self.get_node_option(node_key)?.ok_or_else(|| Self::Error::from(format!("Missing node at {:?}.", node_key)))
    /// ```
    fn get_node(&self, node_key: &NodeKey<N>) -> Result<Node<K, H, N>, Self::Error>;

    /// Gets node given a node key. Returns `None` if the node does not exist.
    fn get_node_option(&self, node_key: &NodeKey<N>) -> Result<Option<Node<K, H, N>>, Self::Error>;

    /// Gets a value given a key. Returns `None` if the value does not exist.
    // TODO(@preston-evans98): Make the return type cheaply cloneable
    fn get_value(&self, key: &(Version, K)) -> Result<Option<Vec<u8>>, Self::Error>;

    /// Gets the rightmost leaf at a version. Note that this assumes we are in the process of
    /// restoring the tree and all nodes are at the same version.
    fn get_rightmost_leaf(
        &self,
        version: Version,
    ) -> Result<Option<(NodeKey<N>, LeafNode<K, H, N>)>, Self::Error>;
}

pub trait PhysicalTreeReader<K> {
    type Error: Into<anyhow::Error> + Send + Sync + 'static;
    fn get_physical_node(&self, node_key: &PhysicalNodeKey)
        -> Result<PhysicalNode<K>, Self::Error>;

    fn get_physical_node_option(
        &self,
        node_key: &PhysicalNodeKey,
    ) -> Result<Option<PhysicalNode<K>>, Self::Error>;

    // TODO(@preston-evans98): Make the return type cheaply cloneable
    fn get_value(&self, key: &(Version, K)) -> Result<Option<Vec<u8>>, Self::Error>;

    /// Gets the rightmost leaf at a version. Note that this assumes we are in the process of
    /// restoring the tree and all nodes are at the same version.
    fn get_rightmost_physical_leaf(
        &self,
        version: Version,
    ) -> Result<Option<(PhysicalNodeKey, PhysicalLeafNode<K>)>, Self::Error>;
}

/// Node batch that will be written into db atomically with other batches.
pub type NodeBatch<K, H, const N: usize> = HashMap<NodeKey<N>, Node<K, H, N>>;

pub trait TreeWriter<K, H, const N: usize>: Send + Sync {
    type Error: std::error::Error + Send + Sync;
    fn write_node_batch(&self, node_batch: &NodeBatch<K, H, N>) -> Result<(), Self::Error>;
}

/// The hash of a key
#[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
pub struct KeyHash<const N: usize>(pub HashOutput<N>);

impl<const N: usize> KeyHash<N> {
    pub fn nibble(&self, index: usize) -> u8 {
        self.0.nibble(index)
    }
    pub fn iter_bits(&self) -> HashValueBitIterator<N> {
        self.0.iter_bits()
    }

    pub fn common_prefix_bits_len(&self, other: &Self) -> usize {
        self.0.common_prefix_bits_len(other.0)
    }
}

impl<const N: usize> NibbleExt<N> for KeyHash<N> {
    fn get_nibble(&self, index: usize) -> Nibble {
        self.0.get_nibble(index)
    }

    fn common_prefix_nibbles_len(&self, other: HashOutput<N>) -> usize {
        self.0.common_prefix_nibbles_len(other)
    }
}

impl<const N: usize> std::fmt::Display for KeyHash<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// The hash of a value
#[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
pub struct ValueHash<const N: usize>(pub HashOutput<N>);

impl<const N: usize> std::fmt::Display for ValueHash<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}
/// The hash of a node in the JMT. Alias for HashOutput<N>
pub type NodeHash<const N: usize> = HashOutput<N>;

/// Indicates a node becomes stale since `stale_since_version`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
pub struct StaleNodeIndex<const N: usize> {
    /// The version since when the node is overwritten and becomes stale.
    pub stale_since_version: Version,
    /// The [`NodeKey`](node_type/struct.NodeKey.html) identifying the node associated with this
    /// record.
    pub node_key: NodeKey<N>,
}

/// This is a wrapper of [`NodeBatch`](type.NodeBatch.html),
/// [`StaleNodeIndexBatch`](type.StaleNodeIndexBatch.html) and some stats of nodes that represents
/// the incremental updates of a tree and pruning indices after applying a write set,
/// which is a vector of `hashed_account_address` and `new_value` pairs.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct TreeUpdateBatch<K: Key, H, const N: usize> {
    pub node_batch: Vec<Vec<(NodeKey<N>, Node<K, H, N>)>>,
    pub stale_node_index_batch: Vec<Vec<StaleNodeIndex<N>>>,
    pub num_new_leaves: usize,
    pub num_stale_leaves: usize,
}

impl<K, H, const N: usize> TreeUpdateBatch<K, H, N>
where
    K: Key,
    H: TreeHash<N>,
{
    pub fn new() -> Self {
        Self {
            node_batch: vec![vec![]],
            stale_node_index_batch: vec![vec![]],
            num_new_leaves: 0,
            num_stale_leaves: 0,
        }
    }

    pub fn combine(&mut self, other: Self) {
        let Self {
            node_batch,
            stale_node_index_batch,
            num_new_leaves,
            num_stale_leaves,
        } = other;

        self.node_batch.extend(node_batch);
        self.stale_node_index_batch.extend(stale_node_index_batch);
        self.num_new_leaves += num_new_leaves;
        self.num_stale_leaves += num_stale_leaves;
    }

    #[cfg(any(test, feature = "fuzzing"))]
    pub fn num_stale_node(&self) -> usize {
        self.stale_node_index_batch.iter().map(Vec::len).sum()
    }

    fn inc_num_new_leaves(&mut self) {
        self.num_new_leaves += 1;
    }

    fn inc_num_stale_leaves(&mut self) {
        self.num_stale_leaves += 1;
    }

    pub fn put_node(&mut self, node_key: NodeKey<N>, node: Node<K, H, N>) {
        if node.is_leaf() {
            self.inc_num_new_leaves();
        }
        self.node_batch[0].push((node_key, node))
    }

    pub fn put_stale_node(
        &mut self,
        node_key: NodeKey<N>,
        stale_since_version: Version,
        node: &Node<K, H, N>,
    ) {
        if node.is_leaf() {
            self.inc_num_stale_leaves();
        }
        self.stale_node_index_batch[0].push(StaleNodeIndex {
            node_key,
            stale_since_version,
        });
    }
}

/// An iterator that iterates the index range (inclusive) of each different nibble at given
/// `nibble_idx` of all the keys in a sorted key-value pairs which have the identical HashValue
/// prefix (up to nibble_idx).
pub struct NibbleRangeIterator<'a, V, const N: usize> {
    sorted_kvs: &'a [(KeyHash<N>, V)],
    nibble_idx: usize,
    pos: usize,
}

impl<'a, V, const N: usize> NibbleRangeIterator<'a, V, N> {
    fn new(sorted_kvs: &'a [(KeyHash<N>, V)], nibble_idx: usize) -> Self {
        assert!(nibble_idx < HashOutput::<N>::ROOT_NIBBLE_HEIGHT);
        NibbleRangeIterator {
            sorted_kvs,
            nibble_idx,
            pos: 0,
        }
    }
}

impl<'a, V, const N: usize> std::iter::Iterator for NibbleRangeIterator<'a, V, N> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let left = self.pos;
        if self.pos < self.sorted_kvs.len() {
            let cur_nibble: u8 = self.sorted_kvs[left].0.nibble(self.nibble_idx);
            let (mut i, mut j) = (left, self.sorted_kvs.len() - 1);
            // Find the last index of the cur_nibble.
            while i < j {
                let mid = j - (j - i) / 2;
                if self.sorted_kvs[mid].0.nibble(self.nibble_idx) > cur_nibble {
                    j = mid - 1;
                } else {
                    i = mid;
                }
            }
            self.pos = i + 1;
            Some((left, i))
        } else {
            None
        }
    }
}

#[derive(Debug, Error)]
pub enum JmtError<E> {
    #[error("Invalid null")]
    InvalidNull,
    #[error(transparent)]
    ReaderError(#[from] E),
    #[error("ran out of nibbles searching for key {0:?}")]
    PathTooShort(Vec<u8>),
    #[error("The JMT contains a cycle!")]
    ContainsCycle,
    #[error("Missing key")]
    MissingKey,
    #[error("Cannot find root for version {version:}. Probably pruned")]
    MissingRoot { version: u64 },
    #[error(transparent)]
    CodecError(CodecError),
    #[error(transparent)]
    AnyhowError(anyhow::Error),
}

/// The Jellyfish Merkle tree data structure. See [`crate`] for description.
pub struct JellyfishMerkleTree<'a, R, K, H, const N: usize> {
    reader: &'a R,
    phantom_key: PhantomData<K>,
    phantom_hasher: PhantomData<H>,
}

impl<'a, R, K, H, const N: usize> JellyfishMerkleTree<'a, R, K, H, N>
where
    R: 'a + TreeReader<K, H, N> + Sync,
    K: Key,
    H: TreeHash<N>,
{
    /// Creates a `JellyfishMerkleTree` backed by the given [`TreeReader`](trait.TreeReader.html).
    pub fn new(reader: &'a R) -> Self {
        Self {
            reader,
            phantom_key: PhantomData,
            phantom_hasher: PhantomData,
        }
    }

    /// Get the node hash from the cache if cache is provided, otherwise (for test only) compute it.
    fn get_hash(
        node_key: &NodeKey<N>,
        node: &Node<K, H, N>,
        hash_cache: &Option<&HashMap<NibblePath<N>, NodeHash<N>>>,
    ) -> NodeHash<N> {
        if let Some(cache) = hash_cache {
            match cache.get(node_key.nibble_path()) {
                Some(hash) => *hash,
                None => unreachable!("{:?} can not be found in hash cache", node_key),
            }
        } else {
            node.hash()
        }
    }

    /// For each value set:
    /// Returns the new nodes and values in a batch after applying `value_set`. For
    /// example, if after transaction `T_i` the committed state of tree in the persistent storage
    /// looks like the following structure:
    ///
    /// ```text
    ///              S_i
    ///             /   \
    ///            .     .
    ///           .       .
    ///          /         \
    ///         o           x
    ///        / \
    ///       A   B
    ///        storage (disk)
    /// ```
    ///
    /// where `A` and `B` denote the states of two adjacent accounts, and `x` is a sibling subtree
    /// of the path from root to A and B in the tree. Then a `value_set` produced by the next
    /// transaction `T_{i+1}` modifies other accounts `C` and `D` exist in the subtree under `x`, a
    /// new partial tree will be constructed in memory and the structure will be:
    ///
    /// ```text
    ///                 S_i      |      S_{i+1}
    ///                /   \     |     /       \
    ///               .     .    |    .         .
    ///              .       .   |   .           .
    ///             /         \  |  /             \
    ///            /           x | /               x'
    ///           o<-------------+-               / \
    ///          / \             |               C   D
    ///         A   B            |
    ///           storage (disk) |    cache (memory)
    /// ```
    ///
    /// With this design, we are able to query the global state in persistent storage and
    /// generate the proposed tree delta based on a specific root hash and `value_set`. For
    /// example, if we want to execute another transaction `T_{i+1}'`, we can use the tree `S_i` in
    /// storage and apply the `value_set` of transaction `T_{i+1}`. Then if the storage commits
    /// the returned batch, the state `S_{i+1}` is ready to be read from the tree by calling
    /// [`get_with_proof`](struct.JellyfishMerkleTree.html#method.get_with_proof). Anything inside
    /// the batch is not reachable from public interfaces before being committed.
    pub fn batch_put_value_set(
        &self,
        value_set: Vec<(KeyHash<N>, Option<&(ValueHash<N>, K)>)>,
        node_hashes: Option<&HashMap<NibblePath<N>, NodeHash<N>>>,
        persisted_version: Option<Version>,
        version: Version,
    ) -> Result<(NodeHash<N>, TreeUpdateBatch<K, H, N>), JmtError<R::Error>> {
        let deduped_and_sorted_kvs = value_set
            .into_iter()
            .collect::<BTreeMap<_, _>>()
            .into_iter()
            .collect::<Vec<_>>();

        let mut batch = TreeUpdateBatch::new();
        let root_node_opt = if let Some(persisted_version) = persisted_version {
            run_on_io_pool_if_enabled(|| {
                self.batch_insert_at(
                    &NodeKey::new_empty_path(persisted_version),
                    version,
                    deduped_and_sorted_kvs.as_slice(),
                    0,
                    &node_hashes,
                    &mut batch,
                )
            })?
        } else {
            self.create_subtree_from_batch(
                &NodeKey::new_empty_path(version),
                version,
                deduped_and_sorted_kvs.as_slice(),
                0,
                &node_hashes,
                &mut batch,
            )?
        };

        let node_key = NodeKey::new_empty_path(version);
        let root_hash = if let Some(root_node) = root_node_opt {
            set_leaf_count_if_enabled(root_node.leaf_count());
            let hash = root_node.hash();
            batch.put_node(node_key, root_node);
            hash
        } else {
            set_leaf_count_if_enabled(0);
            batch.put_node(node_key, Node::Null);
            H::SPARSE_MERKLE_PLACEHOLDER_HASH
        };

        Ok((root_hash, batch))
    }

    /// Insert a slice of changes into the (sub)tree rooted at `node_key`, keeping a record of updates
    /// in the provided [`TreeUpdateBatch`]
    fn batch_insert_at(
        &self,
        node_key: &NodeKey<N>,
        version: Version,
        kvs: &[(KeyHash<N>, Option<&(ValueHash<N>, K)>)],
        depth: usize,
        hash_cache: &Option<&HashMap<NibblePath<N>, NodeHash<N>>>,
        batch: &mut TreeUpdateBatch<K, H, N>,
    ) -> Result<Option<Node<K, H, N>>, JmtError<R::Error>> {
        let node = self.reader.get_node(node_key)?;
        batch.put_stale_node(node_key.clone(), version, &node);

        match node {
            Node::Internal(internal_node) => {
                let range_iter = NibbleRangeIterator::new(kvs, depth);
                // Build the children of this node by recursively inserting.
                // Equivalent to calling
                //  range_iter.map(|(left, right)| self.insert_at_child(..., left, right, ..., batch))
                //   .collect::<Result<_, JmtError<R::Error>>>()
                let new_children: Vec<_> = parallel_process_range_if_enabled::<R, K, H, _, N>(
                    depth,
                    range_iter,
                    batch,
                    |left: usize, right: usize, batch_ref: &mut TreeUpdateBatch<K, H, N>| {
                        self.insert_at_child(
                            node_key,
                            &internal_node,
                            version,
                            kvs,
                            left,
                            right,
                            depth,
                            hash_cache,
                            batch_ref,
                        )
                    },
                )?;

                let mut old_children: Children<N> = internal_node.into();
                let mut new_created_children = HashMap::new();

                // A nibble is only present in `new_children` if a key was modified that contained that nibble
                for (child_nibble, child_option) in new_children {
                    // If a node at that position was created, insert it into new `new_created_children`
                    if let Some(child) = child_option {
                        new_created_children.insert(child_nibble, child);
                    // Otherwise the node was modified and not created - so it must have been deleted.
                    // In that case, we don't need to track it any more, so remove it from "old children" as well.
                    } else {
                        old_children.remove(&child_nibble);
                    }
                }

                // If there are no leftover "old_children" that we need to track and no new children, this is an empty subtree. Return None
                if old_children.is_empty() && new_created_children.is_empty() {
                    return Ok(None);
                }

                // If there's exactly one *new* child and it's a leaf node, then we might not need a branch node.
                // Check for those special cases:
                if new_created_children.len() == 1 {
                    let (new_nibble, new_child) = new_created_children.iter().next().unwrap();
                    if new_child.is_leaf() {
                        // If there are no old nodes that still need to be tracked, we don't need a branch node
                        if old_children.len() == 0 {
                            return Ok(Some(new_child.clone()));
                        }

                        // If there was exactly one old child *and* it's getting overwritten by the new child (i.e. it's at the same index),
                        // then we also don't need a branch node.
                        if old_children.len() == 1 {
                            let (old_nibble, _old_child) = old_children.iter().next().unwrap();
                            if old_nibble == new_nibble && new_child.is_leaf() {
                                return Ok(Some(new_child.clone()));
                            }
                        }
                    }
                }

                // If there is exactly one leaf node leftover from before and we haven't added any new children, then
                // we also don't need a branch node.
                if old_children.len() == 1 && new_created_children.len() == 0 {
                    let (old_child_nibble, old_child) =
                        old_children.iter().next().expect("must exist");
                    if old_child.is_leaf() {
                        // We'll re-use the node body, but its location may change, so we add it to the stale
                        // node tracker
                        let old_child_node_key =
                            node_key.gen_child_node_key(old_child.version, *old_child_nibble);
                        let old_child_node = self.reader.get_node(&old_child_node_key)?;
                        batch.put_stale_node(old_child_node_key, version, &old_child_node);

                        return Ok(Some(old_child_node));
                    }
                }

                // If we've reached this point, we need a branch node. Create and return it.
                let mut new_children = old_children;
                for (child_index, new_child_node) in new_created_children {
                    let new_child_node_key = node_key.gen_child_node_key(version, child_index);
                    new_children.insert(
                        child_index,
                        Child::new(
                            Self::get_hash(&new_child_node_key, &new_child_node, hash_cache),
                            version,
                            new_child_node.node_type(),
                        ),
                    );
                    batch.put_node(new_child_node_key, new_child_node);
                }
                let new_internal_node = InternalNode::new(new_children);
                Ok(Some(new_internal_node.into()))
            }
            Node::Leaf(leaf_node) => self.batch_update_subtree_with_existing_leaf(
                node_key, version, leaf_node, kvs, depth, hash_cache, batch,
            ),
            Node::Null => {
                if depth == 0 {
                    return Err(JmtError::InvalidNull);
                }
                self.create_subtree_from_batch(node_key, version, kvs, 0, hash_cache, batch)
            }
        }
    }

    fn insert_at_child(
        &self,
        node_key: &NodeKey<N>,
        internal_node: &InternalNode<H, N>,
        version: Version,
        kvs: &[(KeyHash<N>, Option<&(ValueHash<N>, K)>)],
        left: usize,
        right: usize,
        depth: usize,
        hash_cache: &Option<&HashMap<NibblePath<N>, NodeHash<N>>>,
        batch: &mut TreeUpdateBatch<K, H, N>,
    ) -> Result<(Nibble, Option<Node<K, H, N>>), JmtError<R::Error>> {
        let child_index = kvs[left].0.get_nibble(depth);
        let child = internal_node.child(child_index);

        let new_child_node_option = match child {
            Some(child) => self.batch_insert_at(
                &node_key.gen_child_node_key(child.version, child_index),
                version,
                &kvs[left..=right],
                depth + 1,
                hash_cache,
                batch,
            )?,
            None => self.create_subtree_from_batch(
                &node_key.gen_child_node_key(version, child_index),
                version,
                &kvs[left..=right],
                depth + 1,
                hash_cache,
                batch,
            )?,
        };

        Ok((child_index, new_child_node_option))
    }

    /// A helper function that updates a subtree which currently contains a leaf node. If we're inserting any additional nodes,
    /// this will require replacing the leaf with a branch node and re-inserting the leaf below that branch.
    fn batch_update_subtree_with_existing_leaf(
        &self,
        node_key: &NodeKey<N>,
        version: Version,
        existing_leaf_node: LeafNode<K, H, N>,
        kvs: &[(KeyHash<N>, Option<&(ValueHash<N>, K)>)],
        depth: usize,
        hash_cache: &Option<&HashMap<NibblePath<N>, NodeHash<N>>>,
        batch: &mut TreeUpdateBatch<K, H, N>,
    ) -> Result<Option<Node<K, H, N>>, JmtError<R::Error>> {
        let existing_leaf_key = existing_leaf_node.account_key();

        // If we're only inserting one node into this subtree, and it's overwriting the current leaf,
        // then we won't need to do any recursive work. In this case:
        if kvs.len() == 1 && kvs[0].0 == existing_leaf_key {
            // Either make a new leaf node from the new value...
            if let (key, Some((value_hash, state_key))) = kvs[0] {
                let new_leaf_node = Node::new_leaf(key, *value_hash, (state_key.clone(), version));
                return Ok(Some(new_leaf_node));
            // ...or delete the leaf node if no new value was provided
            } else {
                inc_deletion_count_if_enabled(1);
                return Ok(None);
            }
        }

        // If we couldn't return early, then we have some nodes to insert.
        // Iterate over them and figure out which subtree they need to go in
        let existing_leaf_bucket = existing_leaf_key.get_nibble(depth);
        let mut isolated_existing_leaf = true;
        let mut children = vec![];
        for (left, right) in NibbleRangeIterator::new(kvs, depth) {
            let child_index = kvs[left].0.get_nibble(depth);
            let child_node_key = node_key.gen_child_node_key(version, child_index);
            // Some of the nodes might need to get inserted into the subtree containing the existing leaf.
            // In that case, recursively call this function =
            let new_child = if existing_leaf_bucket == child_index {
                isolated_existing_leaf = false;
                self.batch_update_subtree_with_existing_leaf(
                    &child_node_key,
                    version,
                    existing_leaf_node.clone(),
                    &kvs[left..=right],
                    depth + 1,
                    hash_cache,
                    batch,
                )?
            } else {
                // All of the other subtrees are currently empty. Do a standard insertion into each of them.
                self.create_subtree_from_batch(
                    &child_node_key,
                    version,
                    &kvs[left..=right],
                    depth + 1,
                    hash_cache,
                    batch,
                )?
            };
            if let Some(new_child_node) = new_child {
                children.push((child_index, new_child_node));
            }
        }

        // We might not have touched the subtree with the existing leaf. If we didn't,
        // the previous loop won't have added it into `children`, so we need to add it
        // separately.
        if isolated_existing_leaf {
            children.push((existing_leaf_bucket, existing_leaf_node.into()));
        }

        // If there are no children in this subtree, don't make anode
        if children.is_empty() {
            Ok(None)
        // If this subtree contains exactly one leaf, just return the leaf
        } else if children.len() == 1 && children[0].1.is_leaf() {
            let (_, child) = children.pop().expect("Must exist");
            Ok(Some(child))
        } else {
            // Otherwise, build a new branch node and return it
            let new_internal_node = InternalNode::new(
                children
                    .into_iter()
                    .map(|(child_index, new_child_node)| {
                        let new_child_node_key = node_key.gen_child_node_key(version, child_index);
                        let result = (
                            child_index,
                            Child::new(
                                Self::get_hash(&new_child_node_key, &new_child_node, hash_cache),
                                version,
                                new_child_node.node_type(),
                            ),
                        );
                        batch.put_node(new_child_node_key, new_child_node);
                        result
                    })
                    .collect(),
            );
            Ok(Some(new_internal_node.into()))
        }
    }

    /// Create a new subtree where none existed before
    fn create_subtree_from_batch(
        &self,
        node_key: &NodeKey<N>,
        version: Version,
        kvs: &[(KeyHash<N>, Option<&(ValueHash<N>, K)>)],
        depth: usize,
        hash_cache: &Option<&HashMap<NibblePath<N>, NodeHash<N>>>,
        batch: &mut TreeUpdateBatch<K, H, N>,
    ) -> Result<Option<Node<K, H, N>>, JmtError<R::Error>> {
        // If this subtree will only contain a single node, it's either a leaf or Null (if the update was a deletion).
        // Handle those cases and return early
        if kvs.len() == 1 {
            if let (key, Some((value_hash, state_key))) = kvs[0] {
                let new_leaf_node = Node::new_leaf(key, *value_hash, (state_key.clone(), version));
                return Ok(Some(new_leaf_node));
            } else {
                return Ok(None);
            }
        }

        // If we reach this point, we need to recursively insert into subtrees, and then create a node based on the result
        let mut children = vec![];
        for (left, right) in NibbleRangeIterator::new(kvs, depth) {
            let child_index = kvs[left].0.get_nibble(depth);
            let child_node_key = node_key.gen_child_node_key(version, child_index);
            if let Some(new_child_node) = self.create_subtree_from_batch(
                &child_node_key,
                version,
                &kvs[left..=right],
                depth + 1,
                hash_cache,
                batch,
            )? {
                children.push((child_index, new_child_node))
            }
        }
        // If there were no children (all updates were deletions), we don't need to put a node here at all
        if children.is_empty() {
            Ok(None)
        // If there's only a single child and it's a leaf node, we don't need to create an internal node.
        // Just return the leaf.
        } else if children.len() == 1 && children[0].1.is_leaf() {
            let (_, child) = children.pop().expect("Must exist");
            Ok(Some(child))
        } else {
            // Otherwise, we need an internal node. Create and return it.
            let new_internal_node = InternalNode::new(
                children
                    .into_iter()
                    .map(|(child_index, new_child_node)| {
                        let new_child_node_key = node_key.gen_child_node_key(version, child_index);
                        let result = (
                            child_index,
                            Child::new(
                                Self::get_hash(&new_child_node_key, &new_child_node, hash_cache),
                                version,
                                new_child_node.node_type(),
                            ),
                        );
                        batch.put_node(new_child_node_key, new_child_node);
                        result
                    })
                    .collect(),
            );
            Ok(Some(new_internal_node.into()))
        }
    }

    ///
    /// [`put_value_sets`](struct.JellyfishMerkleTree.html#method.put_value_set) without the node hash
    /// cache and assuming the base version is the immediate previous version.
    #[cfg(any(test, feature = "fuzzing"))]
    pub fn put_value_set_test(
        &self,
        value_set: Vec<(KeyHash<N>, Option<&(ValueHash<N>, K)>)>,
        version: Version,
    ) -> Result<(NodeHash<N>, TreeUpdateBatch<K, H, N>), JmtError<R::Error>> {
        self.batch_put_value_set(
            value_set.into_iter().map(|(k, v)| (k, v)).collect(),
            None,
            version.checked_sub(1),
            version,
        )
    }

    /// Returns the value (if applicable) and the corresponding merkle proof.
    pub fn get_with_proof(
        &self,
        key: KeyHash<N>,
        version: Version,
    ) -> Result<
        (
            Option<(ValueHash<N>, (K, Version))>,
            SparseMerkleProof<H, N>,
        ),
        JmtError<R::Error>,
    > {
        self.get_with_proof_ext(key, version)
            .map(|(value, proof_ext)| (value, proof_ext.into()))
    }

    pub fn get_with_proof_ext(
        &self,
        key: KeyHash<N>,
        version: Version,
    ) -> Result<
        (
            Option<(ValueHash<N>, (K, Version))>,
            SparseMerkleProofExt<H, N>,
        ),
        JmtError<R::Error>,
    > {
        // Empty tree just returns proof with no sibling hash.
        let mut next_node_key = NodeKey::new_empty_path(version);
        let mut siblings = vec![];
        let nibble_path = NibblePath::<N>::new_even(key.0.to_vec());
        let mut nibble_iter = nibble_path.nibbles();

        // We limit the number of loops here deliberately to avoid potential cyclic graph bugs
        // in the tree structure.
        for nibble_depth in 0..=NibblePath::<N>::ROOT_NIBBLE_HEIGHT {
            let next_node = self.reader.get_node(&next_node_key).map_err(|err| {
                if nibble_depth == 0 {
                    JmtError::MissingRoot { version }
                } else {
                    JmtError::AnyhowError(err.into())
                }
            })?;
            match next_node {
                Node::Internal(internal_node) => {
                    let queried_child_index = nibble_iter
                        .next()
                        .ok_or_else(|| JmtError::PathTooShort(key.0.to_vec()))?;
                    let (child_node_key, mut siblings_in_internal) = internal_node
                        .get_child_with_siblings(
                            &next_node_key,
                            queried_child_index,
                            Some(self.reader),
                        )
                        .map_err(|e| JmtError::CodecError(e))?;
                    siblings.append(&mut siblings_in_internal);
                    next_node_key = match child_node_key {
                        Some(node_key) => node_key,
                        None => {
                            return Ok((
                                None,
                                SparseMerkleProofExt::new(None, {
                                    siblings.reverse();
                                    siblings
                                }),
                            ))
                        }
                    };
                }
                Node::Leaf(leaf_node) => {
                    return Ok((
                        if leaf_node.account_key() == key {
                            Some((leaf_node.value_hash(), leaf_node.value_index().clone()))
                        } else {
                            None
                        },
                        SparseMerkleProofExt::new(Some(leaf_node.into()), {
                            siblings.reverse();
                            siblings
                        }),
                    ));
                }
                Node::Null => {
                    return Ok((None, SparseMerkleProofExt::new(None, vec![])));
                }
            }
        }
        return Err(JmtError::ContainsCycle);
    }

    /// Gets the proof that shows a list of keys up to `rightmost_key_to_prove` exist at `version`.
    pub fn get_range_proof(
        &self,
        rightmost_key_to_prove: KeyHash<N>,
        version: Version,
    ) -> Result<SparseMerkleRangeProof<H, N>, JmtError<R::Error>> {
        let (account, proof) = self.get_with_proof(rightmost_key_to_prove, version)?;
        if account.is_none() {
            return Err(JmtError::MissingKey);
        }

        let siblings = proof
            .siblings()
            .iter()
            .rev()
            .zip(rightmost_key_to_prove.0.iter_bits())
            .filter_map(|(sibling, bit)| {
                // We only need to keep the siblings on the right.
                if !bit {
                    Some(*sibling)
                } else {
                    None
                }
            })
            .rev()
            .collect();
        Ok(SparseMerkleRangeProof::new(siblings))
    }

    #[cfg(any(test, feature = "fuzzing"))]
    pub fn get(
        &self,
        key: KeyHash<N>,
        version: Version,
    ) -> Result<Option<ValueHash<N>>, JmtError<R::Error>> {
        Ok(self.get_with_proof(key, version)?.0.map(|x| x.0))
    }

    fn get_root_node(&self, version: Version) -> Result<Node<K, H, N>, JmtError<R::Error>> {
        self.get_root_node_option(version)?
            .ok_or_else(|| JmtError::MissingRoot { version })
    }

    fn get_root_node_option(
        &self,
        version: Version,
    ) -> Result<Option<Node<K, H, N>>, JmtError<R::Error>> {
        let root_node_key = NodeKey::new_empty_path(version);
        self.reader
            .get_node_option(&root_node_key)
            .map_err(|e| JmtError::AnyhowError(e.into()))
    }

    pub fn get_root_hash(&self, version: Version) -> Result<NodeHash<N>, JmtError<R::Error>> {
        self.get_root_node(version).map(|n| n.hash())
    }

    pub fn get_root_hash_option(
        &self,
        version: Version,
    ) -> Result<Option<NodeHash<N>>, JmtError<R::Error>> {
        Ok(self.get_root_node_option(version)?.map(|n| n.hash()))
    }

    pub fn get_leaf_count(&self, version: Version) -> Result<usize, JmtError<R::Error>> {
        self.get_root_node(version).map(|n| n.leaf_count())
    }

    pub fn get_all_nodes_referenced(
        &self,
        version: Version,
    ) -> Result<Vec<NodeKey<N>>, JmtError<R::Error>> {
        let mut out_keys = vec![];
        self.get_all_nodes_referenced_impl(NodeKey::new_empty_path(version), &mut out_keys)?;
        Ok(out_keys)
    }

    fn get_all_nodes_referenced_impl(
        &self,
        key: NodeKey<N>,
        out_keys: &mut Vec<NodeKey<N>>,
    ) -> Result<(), JmtError<R::Error>> {
        match self.reader.get_node(&key)? {
            Node::Internal(internal_node) => {
                for (child_nibble, child) in internal_node.children_sorted() {
                    self.get_all_nodes_referenced_impl(
                        key.gen_child_node_key(child.version, *child_nibble),
                        out_keys,
                    )?;
                }
            }
            Node::Leaf(_) | Node::Null => {}
        };

        out_keys.push(key);
        Ok(())
    }
}

trait NibbleExt<const N: usize> {
    fn get_nibble(&self, index: usize) -> Nibble;
    fn common_prefix_nibbles_len(&self, other: HashOutput<N>) -> usize;
}

impl<const N: usize> NibbleExt<N> for HashOutput<N> {
    /// Returns the `index`-th nibble.
    fn get_nibble(&self, index: usize) -> Nibble {
        Nibble::from(if index % 2 == 0 {
            self[index / 2] >> 4
        } else {
            self[index / 2] & 0x0F
        })
    }

    /// Returns the length of common prefix of `self` and `other` in nibbles.
    fn common_prefix_nibbles_len(&self, other: HashOutput<N>) -> usize {
        self.common_prefix_bits_len(other) / 4
    }
}
#[cfg(any(test, feature = "fuzzing"))]
pub mod test_utils {
    use proptest::prelude::Arbitrary;
    use std::hash::Hash;
    use tiny_keccak::{Hasher, Sha3};

    use crate::{
        hash::{CryptoHasher, HashOutput, TreeHash},
        Key,
    };

    /// `TestKey` defines the types of data that can be stored in a Jellyfish Merkle tree and used in
    /// tests.

    pub trait TestKey:
        Key + Arbitrary + std::fmt::Debug + Eq + Hash + Ord + PartialOrd + PartialEq + 'static
    {
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TestHash;

    impl TreeHash<32> for TestHash {
        type Hasher = TestHasher;

        const SPARSE_MERKLE_PLACEHOLDER_HASH: crate::hash::HashOutput<32> =
            HashOutput::new(*b"SPARSE_MERKLE_PLACEHOLDER_HASH\0\0");
    }

    #[derive(Clone)]
    pub struct TestHasher(pub Sha3);

    impl CryptoHasher<32> for TestHasher {
        fn new() -> Self {
            Self(Sha3::v256())
        }

        fn update(mut self, data: &[u8]) -> Self {
            self.0.update(data);
            self
        }

        fn finalize(self) -> crate::hash::HashOutput<32> {
            let mut out = [0u8; 32];
            self.0.finalize(&mut out);
            HashOutput::new(out)
        }
    }
}

#[cfg(test)]
mod test {
    use borsh::BorshSerialize;
    use proptest::prelude::Arbitrary;

    use crate::{hash::HashOutput, types::nibble::Nibble, Key};

    use super::NibbleExt;
    type TestHashValue = HashOutput<TEST_HASH_LENGTH>;
    const TEST_HASH_LENGTH: usize = 32;

    /// `TestValue` defines the types of data that can be stored in a Jellyfish Merkle tree and used in
    /// tests.
    #[cfg(any(test, feature = "fuzzing"))]
    pub trait TestValue: Key + Arbitrary + std::fmt::Debug + Eq + PartialEq + 'static {}

    /// Provides a test_only_hash() method that can be used in tests on types that implement
    /// `serde::Serialize`.
    ///
    /// # Example
    /// ```ignore
    /// b"hello world".test_only_hash();
    /// ```
    pub trait TestOnlyHash {
        /// Generates a hash used only for tests.
        fn test_only_hash(&self) -> TestHashValue;
    }

    impl<T: BorshSerialize + ?Sized> TestOnlyHash for T {
        fn test_only_hash(&self) -> TestHashValue {
            let bytes = borsh::to_vec(self).expect("serialize failed during hash.");
            HashOutput::sha3_256_of(&bytes)
        }
    }

    #[test]
    fn test_common_prefix_nibbles_len() {
        {
            let hash1 = b"hello".test_only_hash();
            let hash2 = b"HELLO".test_only_hash();
            assert_eq!(hash1[0], 0b0011_0011);
            assert_eq!(hash2[0], 0b1011_1000);
            assert_eq!(hash1.common_prefix_nibbles_len(hash2), 0);
        }
        {
            let hash1 = b"hello".test_only_hash();
            let hash2 = b"world".test_only_hash();
            assert_eq!(hash1[0], 0b0011_0011);
            assert_eq!(hash2[0], 0b0100_0010);
            assert_eq!(hash1.common_prefix_nibbles_len(hash2), 0);
        }
        {
            let hash1 = b"hello".test_only_hash();
            let hash2 = b"100011001000".test_only_hash();
            assert_eq!(hash1[0], 0b0011_0011);
            assert_eq!(hash2[0], 0b0011_0011);
            assert_eq!(hash1[1], 0b0011_1000);
            assert_eq!(hash2[1], 0b0010_0010);
            assert_eq!(hash1.common_prefix_nibbles_len(hash2), 2);
        }
        {
            let hash1 = b"hello".test_only_hash();
            let hash2 = b"hello".test_only_hash();
            assert_eq!(
                hash1.common_prefix_nibbles_len(hash2),
                TestHashValue::LENGTH * 2
            );
        }
    }

    #[test]
    fn test_get_nibble() {
        let hash = b"hello".test_only_hash();
        assert_eq!(hash.get_nibble(0), Nibble::from(3));
        assert_eq!(hash.get_nibble(1), Nibble::from(3));
        assert_eq!(hash.get_nibble(2), Nibble::from(3));
        assert_eq!(hash.get_nibble(3), Nibble::from(8));
        assert_eq!(hash.get_nibble(62), Nibble::from(9));
        assert_eq!(hash.get_nibble(63), Nibble::from(2));
    }
}
