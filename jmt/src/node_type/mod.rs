use std::{
    any::type_name,
    collections::HashMap,
    io::{Cursor, Read, Seek, SeekFrom, Write},
    mem::size_of,
};

use crate::{
    errors::{
        self,
        CodecError::{self, InvalidNibblePathLength, InvalidNibblePathPadding},
        InternalNodeConstructionError, NodeDecodeError,
    },
    hash::{CryptoHasher, HashOutput, TreeHash},
    metrics::{inc_internal_encoded_bytes_if_enabled, inc_leaf_encoded_bytes_if_enabled},
    proof::{NodeInProof, SparseMerkleLeafNode},
    types::nibble::{
        nibble_path::{NibblePath, PhysicalNibblePath},
        Nibble,
    },
    KeyHash, TreeReader, ValueHash, Version,
};

use byteorder::{BigEndian, LittleEndian, ReadBytesExt, WriteBytesExt};
#[cfg(any(test, feature = "fuzzing"))]
use proptest::{
    collection::hash_map,
    prelude::any,
    prelude::Arbitrary,
    prop_oneof,
    strategy::{BoxedStrategy, Just, Strategy},
};
#[cfg(any(test, feature = "fuzzing"))]
use proptest_derive::Arbitrary;
use serde::{Deserialize, Serialize};

/// The unique key of each node.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
pub struct NodeKey<const N: usize> {
    // The version at which the node is created.
    version: Version,
    // The nibble path this node represents in the tree.
    nibble_path: NibblePath<N>,
}

impl<const N: usize> Into<PhysicalNodeKey> for NodeKey<N> {
    fn into(self) -> PhysicalNodeKey {
        PhysicalNodeKey {
            version: self.version,
            nibble_path: self.nibble_path.into(),
        }
    }
}

#[derive(Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
#[cfg_attr(
    any(test, feature = "borsh"),
    derive(::borsh::BorshDeserialize, ::borsh::BorshSerialize)
)]
/// A type-erased [`NodeKey`] - with no knowledge of the JMTs hash function or digest size.
/// Allows the creation of database abstractions without excessive generics.
pub struct PhysicalNodeKey {
    // The version at which the node is created.
    version: Version,
    // The nibble path this node represents in the tree.
    nibble_path: PhysicalNibblePath,
}

impl<const N: usize> TryFrom<PhysicalNodeKey> for NodeKey<N> {
    type Error = CodecError;

    fn try_from(value: PhysicalNodeKey) -> Result<Self, Self::Error> {
        Ok(Self {
            version: value.version,
            nibble_path: value.nibble_path.try_into()?,
        })
    }
}

impl PhysicalNodeKey {
    pub fn version(&self) -> Version {
        self.version
    }

    pub fn nibble_path(&self) -> &PhysicalNibblePath {
        &self.nibble_path
    }
    pub fn unpack(self) -> (Version, PhysicalNibblePath) {
        (self.version, self.nibble_path)
    }
}

impl<const N: usize> NodeKey<N> {
    /// Creates a new `NodeKey`.
    pub fn new(version: Version, nibble_path: NibblePath<N>) -> Self {
        Self {
            version,
            nibble_path,
        }
    }

    /// A shortcut to generate a node key consisting of a version and an empty nibble path.
    pub fn new_empty_path(version: Version) -> Self {
        Self::new(version, NibblePath::new_even(vec![]))
    }

    /// Gets the version.
    pub fn version(&self) -> Version {
        self.version
    }

    /// Gets the nibble path.
    pub fn nibble_path(&self) -> &NibblePath<N> {
        &self.nibble_path
    }

    /// Generates a child node key based on this node key.
    pub fn gen_child_node_key(&self, version: Version, n: Nibble) -> Self {
        let mut node_nibble_path = self.nibble_path().clone();
        node_nibble_path.push(n);
        Self::new(version, node_nibble_path)
    }

    /// Generates parent node key at the same version based on this node key.
    pub fn gen_parent_node_key(&self) -> Self {
        let mut node_nibble_path = self.nibble_path().clone();
        assert!(
            node_nibble_path.pop().is_some(),
            "Current node key is root.",
        );
        Self::new(self.version, node_nibble_path)
    }

    /// Sets the version to the given version.
    pub fn set_version(&mut self, version: Version) {
        self.version = version;
    }

    /// Serializes to bytes for physical storage enforcing the same order as that in memory.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = vec![];
        out.write_u64::<BigEndian>(self.version())?;
        out.write_u8(self.nibble_path().num_nibbles() as u8)?;
        out.write_all(self.nibble_path().bytes())?;
        Ok(out)
    }

    /// Recovers from serialized bytes in physical storage.
    pub fn decode(val: &[u8]) -> Result<NodeKey<N>, CodecError> {
        let mut reader = Cursor::new(val);
        let version = reader.read_u64::<BigEndian>()?;
        let num_nibbles = reader.read_u8()? as usize;
        if !num_nibbles <= HashOutput::<N>::ROOT_NIBBLE_HEIGHT {
            return Err(CodecError::NibblePathTooLong {
                max: HashOutput::<N>::ROOT_NIBBLE_HEIGHT,
                got: num_nibbles,
            });
        }
        let mut nibble_bytes = Vec::with_capacity((num_nibbles + 1) / 2);
        reader.read_to_end(&mut nibble_bytes)?;
        if !(num_nibbles + 1) / 2 == nibble_bytes.len() {
            return Err(InvalidNibblePathLength {
                expected: num_nibbles,
                found: nibble_bytes,
            });
        }

        let nibble_path = if num_nibbles % 2 == 0 {
            NibblePath::new_even(nibble_bytes)
        } else {
            let padding = nibble_bytes.last().unwrap() & 0x0F;
            if padding != 0 {
                return Err(InvalidNibblePathPadding { got: padding });
            }

            NibblePath::new_odd(nibble_bytes)
        };
        Ok(NodeKey::new(version, nibble_path))
    }

    pub fn unpack(self) -> (Version, NibblePath<N>) {
        (self.version, self.nibble_path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    any(test, feature = "borsh"),
    derive(::borsh::BorshDeserialize, ::borsh::BorshSerialize)
)]
pub enum NodeType {
    Leaf,
    Null,
    /// A internal node that haven't been finished the leaf count migration, i.e. None or not all
    /// of the children leaf counts are known.
    Internal {
        leaf_count: usize,
    },
}

#[cfg(any(test, feature = "fuzzing"))]
impl Arbitrary for NodeType {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_args: ()) -> Self::Strategy {
        prop_oneof![
            Just(NodeType::Leaf),
            (2..100usize).prop_map(|leaf_count| NodeType::Internal { leaf_count })
        ]
        .boxed()
    }
}

/// Each child of [`InternalNode`] encapsulates a nibble forking at this node.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
pub struct Child<const N: usize> {
    /// The hash value of this child node.
    pub hash: HashOutput<N>,
    /// `version`, the `nibble_path` of the [`NodeKey`] of this [`InternalNode`] the child belongs
    /// to and the child's index constitute the [`NodeKey`] to uniquely identify this child node
    /// from the storage. Used by `[`NodeKey::gen_child_node_key`].
    pub version: Version,
    /// Indicates if the child is a leaf, or if it's an internal node, the total number of leaves
    /// under it (though it can be unknown during migration).
    pub node_type: NodeType,
}

impl<const N: usize> Into<PhysicalChild> for Child<N> {
    fn into(self) -> PhysicalChild {
        PhysicalChild {
            hash: self.hash.to_vec(),
            version: self.version,
            node_type: self.node_type,
        }
    }
}

/// A type-erased [`Child`] - with no knowledge of the JMTs hash function or digest size.
/// Allows the creation of database abstractions without excessive generics.
///
/// Introduces a slight inefficiency, since "hash" values have to be copied to transform from
/// Vec to array types on conversion to [`Child`], but the performance impace should be negligble.
#[derive(Debug, Eq, PartialEq, Clone)]
#[cfg_attr(
    any(test, feature = "borsh"),
    derive(::borsh::BorshDeserialize, ::borsh::BorshSerialize)
)]
pub struct PhysicalChild {
    /// The hash value of this child node.
    hash: Vec<u8>,
    /// `version`, the `nibble_path` of the [`NodeKey`] of this [`InternalNode`] the child belongs
    /// to and the child's index constitute the [`NodeKey`] to uniquely identify this child node
    /// from the storage. Used by `[`NodeKey::gen_child_node_key`].
    version: Version,
    /// Indicates if the child is a leaf, or if it's an internal node, the total number of leaves
    /// under it (though it can be unknown during migration).
    node_type: NodeType,
}

impl<const N: usize> TryFrom<PhysicalChild> for Child<N> {
    type Error = CodecError;

    fn try_from(value: PhysicalChild) -> Result<Self, Self::Error> {
        Ok(Self {
            hash: HashOutput::from_slice(value.hash)?,
            version: value.version,
            node_type: value.node_type,
        })
    }
}

impl<const N: usize> Child<N> {
    pub fn new(hash: HashOutput<N>, version: Version, node_type: NodeType) -> Self {
        Self {
            hash,
            version,
            node_type,
        }
    }

    pub fn is_leaf(&self) -> bool {
        matches!(self.node_type, NodeType::Leaf)
    }

    pub fn leaf_count(&self) -> usize {
        match self.node_type {
            NodeType::Leaf => 1,
            NodeType::Internal { leaf_count } => leaf_count,
            NodeType::Null => unreachable!("Child cannot be Null"),
        }
    }
}

/// [`Children`] is just a collection of children belonging to a [`InternalNode`], indexed from 0 to
/// 15, inclusive.
// TODO(preston-evans98): change this to a Vec of tuples for better performance
pub(crate) type Children<const N: usize> = HashMap<Nibble, Child<N>>;
pub(crate) type PartialChildren = HashMap<Nibble, PhysicalChild>;

/// Represents a 4-level subtree with 16 children at the bottom level. Theoretically, this reduces
/// IOPS to query a tree by 4x since we compress 4 levels in a standard Merkle tree into 1 node.
/// Though we choose the same internal node structure as that of Patricia Merkle tree, the root hash
/// computation logic is similar to a 4-level sparse Merkle tree except for some customizations. See
/// the `CryptoHash` trait implementation below for details.
#[derive(Debug, Eq, PartialEq)]
pub struct InternalNode<H, const N: usize> {
    /// Up to 16 children.
    children: Children<N>,
    /// Total number of leaves under this internal node
    leaf_count: usize,
    phantom_hasher: std::marker::PhantomData<H>,
}

impl<H, const N: usize> Into<PhysicalInternalNode> for InternalNode<H, N> {
    fn into(self) -> PhysicalInternalNode {
        PhysicalInternalNode {
            children: self
                .children
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            leaf_count: self.leaf_count,
        }
    }
}

// Derive is broken. See comment on SparseMerkleLeafNode<H, const N: usize>
impl<H, const N: usize> Clone for InternalNode<H, N> {
    fn clone(&self) -> Self {
        Self {
            children: self.children.clone(),
            leaf_count: self.leaf_count.clone(),
            phantom_hasher: self.phantom_hasher.clone(),
        }
    }
}

/// A type-erased [`InternalNode`] - with no knowledge of the JMTs hash function or digest size.
/// Allows the creation of database abstractions without excessive generics.
#[derive(Debug, Eq, PartialEq, Clone)]
#[cfg_attr(
    any(test, feature = "borsh"),
    derive(::borsh::BorshDeserialize, ::borsh::BorshSerialize)
)]
pub struct PhysicalInternalNode {
    /// Up to 16 children.
    children: PartialChildren,
    /// Total number of leaves under this internal node
    leaf_count: usize,
}

impl<H, const N: usize> TryFrom<PhysicalInternalNode> for InternalNode<H, N> {
    type Error = CodecError;

    fn try_from(value: PhysicalInternalNode) -> Result<Self, Self::Error> {
        let children: Result<HashMap<Nibble, Child<N>>, CodecError> = value
            .children
            .into_iter()
            .map::<Result<(Nibble, Child<N>), CodecError>, _>(|(k, v)| {
                Ok((k, <PhysicalChild as TryInto<Child<N>>>::try_into(v)?))
            })
            .collect();
        Ok(Self {
            children: children?,
            leaf_count: value.leaf_count,
            phantom_hasher: std::marker::PhantomData,
        })
    }
}

/// Computes the hash of internal node according to [`JellyfishTree`](crate::JellyfishTree)
/// data structure in the logical view. `start` and `nibble_height` determine a subtree whose
/// root hash we want to get. For an internal node with 16 children at the bottom level, we compute
/// the root hash of it as if a full binary Merkle tree with 16 leaves as below:
///
/// ```text
///   4 ->              +------ root hash ------+
///                     |                       |
///   3 ->        +---- # ----+           +---- # ----+
///               |           |           |           |
///   2 ->        #           #           #           #
///             /   \       /   \       /   \       /   \
///   1 ->     #     #     #     #     #     #     #     #
///           / \   / \   / \   / \   / \   / \   / \   / \
///   0 ->   0   1 2   3 4   5 6   7 8   9 A   B C   D E   F
///   ^
/// height
/// ```
///
/// As illustrated above, at nibble height 0, `0..F` in hex denote 16 children hashes.  Each `#`
/// means the hash of its two direct children, which will be used to generate the hash of its
/// parent with the hash of its sibling. Finally, we can get the hash of this internal node.
///
/// However, if an internal node doesn't have all 16 children exist at height 0 but just a few of
/// them, we have a modified hashing rule on top of what is stated above:
/// 1. From top to bottom, a node will be replaced by a leaf child if the subtree rooted at this
/// node has only one child at height 0 and it is a leaf child.
/// 2. From top to bottom, a node will be replaced by the placeholder node if the subtree rooted at
/// this node doesn't have any child at height 0. For example, if an internal node has 3 leaf
/// children at index 0, 3, 8, respectively, and 1 internal node at index C, then the computation
/// graph will be like:
///
/// ```text
///   4 ->              +------ root hash ------+
///                     |                       |
///   3 ->        +---- # ----+           +---- # ----+
///               |           |           |           |
///   2 ->        #           @           8           #
///             /   \                               /   \
///   1 ->     0     3                             #     @
///                                               / \
///   0 ->                                       C   @
///   ^
/// height
/// Note: @ denotes placeholder hash.
/// ```
#[cfg(any(test, feature = "fuzzing"))]
impl<H: TreeHash<N> + 'static, const N: usize> Arbitrary for InternalNode<H, N> {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_args: ()) -> Self::Strategy {
        hash_map(any::<Nibble>(), any::<Child<N>>(), 1..=16)
            .prop_filter(
                "InternalNode constructor panics when its only child is a leaf.",
                |children| {
                    !(children.len() == 1
                        && children.values().next().expect("Must exist.").is_leaf())
                },
            )
            .prop_map(InternalNode::new)
            .boxed()
    }
}

impl<H: TreeHash<N>, const N: usize> InternalNode<H, N> {
    /// Creates a new Internal node.
    pub fn new(children: Children<N>) -> Self {
        Self::new_impl(children).expect("Input children are logical.")
    }

    pub fn new_impl(children: Children<N>) -> Result<Self, InternalNodeConstructionError> {
        // Assert the internal node must have >= 1 children. If it only has one child, it cannot be
        // a leaf node. Otherwise, the leaf node should be a child of this internal node's parent.
        if children.is_empty() {
            return Err(InternalNodeConstructionError::NoChildrenProvided);
        }
        if children.len() == 1 {
            if children
                .values()
                .next()
                .expect("Must have 1 element")
                .is_leaf()
            {
                return Err(InternalNodeConstructionError::OnlyChildIsLeaf);
            }
        }

        let leaf_count = children.values().map(Child::leaf_count).sum();
        Ok(Self {
            children,
            leaf_count,
            phantom_hasher: std::marker::PhantomData,
        })
    }

    pub fn leaf_count(&self) -> usize {
        self.leaf_count
    }

    pub fn node_type(&self) -> NodeType {
        NodeType::Internal {
            leaf_count: self.leaf_count,
        }
    }

    pub fn hash(&self) -> HashOutput<N> {
        self.merkle_hash(
            0,  /* start index */
            16, /* the number of leaves in the subtree of which we want the hash of root */
            self.generate_bitmaps(),
        )
    }

    pub fn children_sorted(&self) -> impl Iterator<Item = (&Nibble, &Child<N>)> {
        let mut sorted = Vec::from_iter(self.children.iter());
        sorted.sort_by_key(|(nibble, _)| **nibble);
        sorted.into_iter()
    }

    pub fn serialize(&self, binary: &mut Vec<u8>) -> Result<(), CodecError> {
        let (mut existence_bitmap, leaf_bitmap) = self.generate_bitmaps();
        binary.write_u16::<LittleEndian>(existence_bitmap)?;
        binary.write_u16::<LittleEndian>(leaf_bitmap)?;
        for _ in 0..existence_bitmap.count_ones() {
            let next_child = existence_bitmap.trailing_zeros() as u8;
            let child = &self.children[&Nibble::from(next_child)];
            serialize_u64_varint(child.version, binary);
            binary.extend(child.hash.to_vec());
            match child.node_type {
                NodeType::Leaf => (),
                NodeType::Internal { leaf_count } => {
                    serialize_u64_varint(leaf_count as u64, binary);
                }
                NodeType::Null => unreachable!("Child cannot be Null"),
            };
            existence_bitmap &= !(1 << next_child);
        }
        Ok(())
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, CodecError> {
        let mut reader = Cursor::new(data);
        let len = data.len();

        // Read and validate existence and leaf bitmaps
        let mut existence_bitmap = reader.read_u16::<LittleEndian>()?;
        let leaf_bitmap = reader.read_u16::<LittleEndian>()?;
        match existence_bitmap {
            0 => return Err(NodeDecodeError::NoChildren.into()),
            _ if (existence_bitmap & leaf_bitmap) != leaf_bitmap => {
                return Err(NodeDecodeError::ExtraLeaves {
                    existing: existence_bitmap,
                    leaves: leaf_bitmap,
                }
                .into())
            }
            _ => (),
        }

        // Reconstruct children
        let mut children = HashMap::new();
        for _ in 0..existence_bitmap.count_ones() {
            let next_child = existence_bitmap.trailing_zeros() as u8;
            let version = deserialize_u64_varint(&mut reader)?;
            let pos = reader.position() as usize;
            let remaining = len - pos;

            if !remaining >= size_of::<HashOutput<N>>() {
                return Err(CodecError::DataTooShort {
                    remaining,
                    desired_type: std::any::type_name::<HashOutput<N>>(),
                    needed: size_of::<HashOutput<N>>(),
                });
            }

            let hash =
                HashOutput::from_slice(&reader.get_ref()[pos..pos + size_of::<HashOutput<N>>()])?;
            reader.seek(SeekFrom::Current(size_of::<HashOutput<N>>() as i64))?;

            let child_bit = 1 << next_child;
            let node_type = if (leaf_bitmap & child_bit) != 0 {
                NodeType::Leaf
            } else {
                let leaf_count = deserialize_u64_varint(&mut reader)? as usize;
                NodeType::Internal { leaf_count }
            };

            children.insert(
                Nibble::from(next_child),
                Child::new(hash, version, node_type),
            );
            existence_bitmap &= !child_bit;
        }
        assert_eq!(existence_bitmap, 0);

        Self::new_impl(children).map_err(|e| e.into())
    }

    /// Gets the `n`-th child.
    pub fn child(&self, n: Nibble) -> Option<&Child<N>> {
        self.children.get(&n)
    }

    /// Generates `existence_bitmap` and `leaf_bitmap` as a pair of `u16`s: child at index `i`
    /// exists if `existence_bitmap[i]` is set; child at index `i` is leaf node if
    /// `leaf_bitmap[i]` is set.
    pub fn generate_bitmaps(&self) -> (u16, u16) {
        let mut existence_bitmap = 0;
        let mut leaf_bitmap = 0;
        for (nibble, child) in self.children.iter() {
            let i = u8::from(*nibble);
            existence_bitmap |= 1u16 << i;
            if child.is_leaf() {
                leaf_bitmap |= 1u16 << i;
            }
        }
        // `leaf_bitmap` must be a subset of `existence_bitmap`.
        assert_eq!(existence_bitmap | leaf_bitmap, existence_bitmap);
        (existence_bitmap, leaf_bitmap)
    }

    /// Given a range [start, start + width), returns the sub-bitmap of that range.
    fn range_bitmaps(start: u8, width: u8, bitmaps: (u16, u16)) -> (u16, u16) {
        assert!(start < 16 && width.count_ones() == 1 && start % width == 0);
        assert!(width <= 16 && (start + width) <= 16);
        // A range with `start == 8` and `width == 4` will generate a mask 0b0000111100000000.
        // use as converting to smaller integer types when 'width == 16'
        let mask = (((1u32 << width) - 1) << start) as u16;
        (bitmaps.0 & mask, bitmaps.1 & mask)
    }

    fn merkle_hash(
        &self,
        start: u8,
        width: u8,
        (existence_bitmap, leaf_bitmap): (u16, u16),
    ) -> HashOutput<N> {
        // Given a bit [start, 1 << nibble_height], return the value of that range.
        let (range_existence_bitmap, range_leaf_bitmap) =
            Self::range_bitmaps(start, width, (existence_bitmap, leaf_bitmap));
        if range_existence_bitmap == 0 {
            // No child under this subtree
            H::SPARSE_MERKLE_PLACEHOLDER_HASH
        } else if width == 1 || (range_existence_bitmap.count_ones() == 1 && range_leaf_bitmap != 0)
        {
            // Only 1 leaf child under this subtree or reach the lowest level
            let only_child_index = Nibble::from(range_existence_bitmap.trailing_zeros() as u8);
            self.child(only_child_index)
                .expect(&format!(
                    "Corrupted internal node: existence_bitmap indicates \
                 the existence of a non-exist child at index {:x}",
                    only_child_index
                ))
                .hash
        } else {
            let left_child = self.merkle_hash(
                start,
                width / 2,
                (range_existence_bitmap, range_leaf_bitmap),
            );
            let right_child = self.merkle_hash(
                start + width / 2,
                width / 2,
                (range_existence_bitmap, range_leaf_bitmap),
            );
            MerkleTreeInternalNode::<H, N>::new(left_child, right_child).hash()
        }
    }

    fn gen_node_in_proof<K: crate::Key, R: TreeReader<K, H, N>>(
        &self,
        start: u8,
        width: u8,
        (existence_bitmap, leaf_bitmap): (u16, u16),
        (tree_reader, node_key): (&R, &NodeKey<N>),
    ) -> Result<NodeInProof<H, N>, CodecError> {
        // Given a bit [start, 1 << nibble_height], return the value of that range.
        let (range_existence_bitmap, range_leaf_bitmap) =
            Self::range_bitmaps(start, width, (existence_bitmap, leaf_bitmap));
        Ok(if range_existence_bitmap == 0 {
            // No child under this subtree
            NodeInProof::Other(H::SPARSE_MERKLE_PLACEHOLDER_HASH)
        } else if width == 1 || (range_existence_bitmap.count_ones() == 1 && range_leaf_bitmap != 0)
        {
            // Only 1 leaf child under this subtree or reach the lowest level
            let only_child_index = Nibble::from(range_existence_bitmap.trailing_zeros() as u8);
            let only_child = self.child(only_child_index).expect(&format!(
                "Corrupted internal node: existence_bitmap indicates \
                         the existence of a non-exist child at index {:x}",
                only_child_index
            ));
            if matches!(only_child.node_type, NodeType::Leaf) {
                let only_child_node_key =
                    node_key.gen_child_node_key(only_child.version, only_child_index);
                match tree_reader.get_node(&only_child_node_key).map_err(|e| {
                    let e: anyhow::Error = e.into();
                    CodecError::NodeFetchError {
                        key: format!("{:?}", &only_child_node_key),
                        err: e.to_string(),
                    }
                })? {
                    Node::Internal(_) => unreachable!(
                        "Corrupted internal node: in-memory leaf child is internal node on disk"
                    ),
                    Node::Leaf(leaf_node) => {
                        NodeInProof::Leaf(SparseMerkleLeafNode::from(leaf_node))
                    }
                    Node::Null => unreachable!("Child cannot be Null"),
                }
            } else {
                NodeInProof::Other(only_child.hash)
            }
        } else {
            let left_child = self.merkle_hash(
                start,
                width / 2,
                (range_existence_bitmap, range_leaf_bitmap),
            );
            let right_child = self.merkle_hash(
                start + width / 2,
                width / 2,
                (range_existence_bitmap, range_leaf_bitmap),
            );
            NodeInProof::Other(MerkleTreeInternalNode::<H, N>::new(left_child, right_child).hash())
        })
    }

    /// Gets the child and its corresponding siblings that are necessary to generate the proof for
    /// the `n`-th child. If it is an existence proof, the returned child must be the `n`-th
    /// child; otherwise, the returned child may be another child. See inline explanation for
    /// details. When calling this function with n = 11 (node `b` in the following graph), the
    /// range at each level is illustrated as a pair of square brackets:
    ///
    /// ```text
    ///     4      [f   e   d   c   b   a   9   8   7   6   5   4   3   2   1   0] -> root level
    ///            ---------------------------------------------------------------
    ///     3      [f   e   d   c   b   a   9   8] [7   6   5   4   3   2   1   0] width = 8
    ///                                  chs <--┘                        shs <--┘
    ///     2      [f   e   d   c] [b   a   9   8] [7   6   5   4] [3   2   1   0] width = 4
    ///                  shs <--┘               └--> chs
    ///     1      [f   e] [d   c] [b   a] [9   8] [7   6] [5   4] [3   2] [1   0] width = 2
    ///                          chs <--┘       └--> shs
    ///     0      [f] [e] [d] [c] [b] [a] [9] [8] [7] [6] [5] [4] [3] [2] [1] [0] width = 1
    ///     ^                chs <--┘   └--> shs
    ///     |   MSB|<---------------------- uint 16 ---------------------------->|LSB
    ///  height    chs: `child_half_start`         shs: `sibling_half_start`
    /// ```
    pub fn get_child_with_siblings<K: crate::Key, R: TreeReader<K, H, N>>(
        &self,
        node_key: &NodeKey<N>,
        n: Nibble,
        reader: Option<&R>,
    ) -> Result<(Option<NodeKey<N>>, Vec<NodeInProof<H, N>>), CodecError> {
        let mut siblings = vec![];
        let (existence_bitmap, leaf_bitmap) = self.generate_bitmaps();

        // Nibble height from 3 to 0.
        for h in (0..4).rev() {
            // Get the number of children of the internal node that each subtree at this height
            // covers.
            let width = 1 << h;
            let (child_half_start, sibling_half_start) = get_child_and_sibling_half_start(n, h);
            // Compute the root hash of the subtree rooted at the sibling of `r`.
            if let Some(reader) = reader {
                siblings.push(self.gen_node_in_proof(
                    sibling_half_start,
                    width,
                    (existence_bitmap, leaf_bitmap),
                    (reader, node_key),
                )?);
            } else {
                siblings.push(
                    self.merkle_hash(sibling_half_start, width, (existence_bitmap, leaf_bitmap))
                        .into(),
                );
            }

            let (range_existence_bitmap, range_leaf_bitmap) =
                Self::range_bitmaps(child_half_start, width, (existence_bitmap, leaf_bitmap));

            if range_existence_bitmap == 0 {
                // No child in this range.
                return Ok((None, siblings));
            } else if width == 1
                || (range_existence_bitmap.count_ones() == 1 && range_leaf_bitmap != 0)
            {
                // Return the only 1 leaf child under this subtree or reach the lowest level
                // Even this leaf child is not the n-th child, it should be returned instead of
                // `None` because it's existence indirectly proves the n-th child doesn't exist.
                // Please read proof format for details.
                let only_child_index = Nibble::from(range_existence_bitmap.trailing_zeros() as u8);
                return Ok((
                    {
                        let only_child_version = self
                            .child(only_child_index)
                            // Should be guaranteed by the self invariants, but these are not easy to express at the moment
                            .expect(&format!(
                                "Corrupted internal node: child_bitmap indicates \
                                     the existence of a non-exist child at index {:x}",
                                only_child_index
                            ))
                            .version;
                        Some(node_key.gen_child_node_key(only_child_version, only_child_index))
                    },
                    siblings,
                ));
            }
        }
        unreachable!("Impossible to get here without returning even at the lowest level.")
    }
}

/// Given a nibble, computes the start position of its `child_half_start` and `sibling_half_start`
/// at `height` level.
pub(crate) fn get_child_and_sibling_half_start(n: Nibble, height: u8) -> (u8, u8) {
    // Get the index of the first child belonging to the same subtree whose root, let's say `r` is
    // at `height` that the n-th child belongs to.
    // Note: `child_half_start` will be always equal to `n` at height 0.
    let child_half_start = (0xFF << height) & u8::from(n);

    // Get the index of the first child belonging to the subtree whose root is the sibling of `r`
    // at `height`.
    let sibling_half_start = child_half_start ^ (1 << height);

    (child_half_start, sibling_half_start)
}

/// Represents an account.
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LeafNode<K, H, const N: usize> {
    // The hashed key associated with this leaf node.
    account_key: KeyHash<N>,
    // The hash of the value.
    value_hash: ValueHash<N>,
    // The key and version that points to the value
    value_index: (K, Version),
    phantom_hasher: std::marker::PhantomData<H>,
}

impl<K, H, const N: usize> Into<PhysicalLeafNode<K>> for LeafNode<K, H, N> {
    fn into(self) -> PhysicalLeafNode<K> {
        PhysicalLeafNode {
            account_key: self.account_key.0.to_vec(),
            value_hash: self.value_hash.0.to_vec(),
            value_index: self.value_index,
        }
    }
}

/// A type-erased [`LeafNode`] - with no knowledge of the JMTs hash function or digest size.
/// Allows the creation of database abstractions without excessive generics.
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone)]
#[cfg_attr(
    any(test, feature = "borsh"),
    derive(::borsh::BorshDeserialize, ::borsh::BorshSerialize)
)]
pub struct PhysicalLeafNode<K> {
    account_key: Vec<u8>,
    value_hash: Vec<u8>,
    value_index: (K, Version),
}

impl<K, H, const N: usize> TryFrom<PhysicalLeafNode<K>> for LeafNode<K, H, N> {
    type Error = errors::CodecError;

    fn try_from(value: PhysicalLeafNode<K>) -> Result<Self, Self::Error> {
        let account_key = KeyHash(HashOutput::from_slice(value.account_key)?);
        let value_hash = ValueHash(HashOutput::from_slice(value.value_hash)?);
        Ok(Self {
            account_key,
            value_hash,
            value_index: value.value_index,
            phantom_hasher: std::marker::PhantomData,
        })
    }
}

// Derive is broken. See comment on SparseMerkleLeafNode<H, const N: usize>
impl<K: Clone, H, const N: usize> Clone for LeafNode<K, H, N> {
    fn clone(&self) -> Self {
        Self {
            account_key: self.account_key.clone(),
            value_hash: self.value_hash.clone(),
            value_index: self.value_index.clone(),
            phantom_hasher: self.phantom_hasher.clone(),
        }
    }
}

impl<K, H, const N: usize> LeafNode<K, H, N>
where
    K: crate::Key,
    H: TreeHash<N>,
{
    /// Creates a new leaf node.
    pub fn new(
        account_key: KeyHash<N>,
        value_hash: ValueHash<N>,
        value_index: (K, Version),
    ) -> Self {
        Self {
            account_key,
            value_hash,
            value_index,
            phantom_hasher: std::marker::PhantomData,
        }
    }

    /// Gets the account key, the hashed account address.
    pub fn account_key(&self) -> KeyHash<N> {
        self.account_key
    }

    /// Gets the associated value hash.
    pub fn value_hash(&self) -> ValueHash<N> {
        self.value_hash
    }

    /// Get the index key to locate the value.
    pub fn value_index(&self) -> &(K, Version) {
        &self.value_index
    }

    pub fn hash(&self) -> HashOutput<N> {
        SparseMerkleLeafNode::<H, N>::new(self.account_key, self.value_hash).hash()
    }

    pub fn serialize(&self, binary: &mut Vec<u8>) -> Result<(), CodecError> {
        binary.extend_from_slice(self.account_key.0.as_ref());
        binary.extend_from_slice(self.value_hash.0.as_ref());
        binary.write_u32::<LittleEndian>(self.value_index.0.key_size() as u32)?;
        binary.extend_from_slice(self.value_index.0.as_ref());
        binary.write_u64::<LittleEndian>(self.value_index.1)?;
        Ok(())
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, CodecError> {
        // Ensure that there is at least enough data for
        // two hashvalues, the key length, and the version
        if data.len() < { (2 * N) + 4 + 8 } {
            return Err(CodecError::DataTooShort {
                remaining: data.len(),
                desired_type: type_name::<Self>(),
                needed: (2 * N) + 4 + 8,
            });
        }
        let account_key = KeyHash(HashOutput::<N>::from_slice(&data[..N])?);
        let value_hash = ValueHash(HashOutput::<N>::from_slice(&data[N..2 * N])?);
        let mut cursor = std::io::Cursor::new(&data[2 * N..]);
        let key_len = cursor.read_u32::<LittleEndian>()?;
        let key_slice = &data[(cursor.position() as usize)..][..key_len as usize];
        let key = K::try_from(key_slice).map_err(|e| CodecError::KeyDecodeError {
            key_type: std::any::type_name::<K>(),
            err: e.to_string(),
        })?;
        cursor.set_position(cursor.position() + key_len as u64);
        let version = cursor.read_u64::<LittleEndian>()?;

        Ok(Self {
            account_key,
            value_hash,
            value_index: (key, version),
            phantom_hasher: std::marker::PhantomData,
        })
        // K::try_from(data[(2 * N) + 4])

        // let account_key = HashValue<N>::des
        // binary.extend_from_slice(self.account_key.as_ref());
        // binary.extend_from_slice(self.value_hash.as_ref());
        // binary.write_u32::<LittleEndian>(self.value_index.0.key_size() as u32);
        // binary.extend_from_slice(self.value_index.0.as_ref());
        // binary.write_u64::<LittleEndian>(self.value_index.1);
        // Ok(())
    }
}

impl<K, H: TreeHash<N>, const N: usize> From<LeafNode<K, H, N>> for SparseMerkleLeafNode<H, N> {
    fn from(leaf_node: LeafNode<K, H, N>) -> Self {
        Self::new(leaf_node.account_key, leaf_node.value_hash)
    }
}

#[repr(u8)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
#[derive(Copy, Clone, PartialEq, Debug, Eq)]
enum NodeTag {
    Leaf = 1,
    Internal = 2,
    Null = 3,
}

impl NodeTag {
    pub fn from_u8(tag: u8) -> Option<Self> {
        Some(match tag {
            1 => Self::Leaf,
            2 => Self::Internal,
            3 => Self::Null,
            _ => return None,
        })
    }

    #[cfg(any(test, feature = "fuzzing"))]
    pub fn to_u8(&self) -> u8 {
        *self as u8
    }
}

/// The concrete node type of [`JellyfishMerkleTree`](crate::JellyfishMerkleTree).
#[derive(Debug, Eq, PartialEq)]
pub enum Node<K, H, const N: usize> {
    /// A wrapper of [`InternalNode`].
    Internal(InternalNode<H, N>),
    /// A wrapper of [`LeafNode`].
    Leaf(LeafNode<K, H, N>),
    /// Represents empty tree only
    Null,
}

impl<K, H, const N: usize> Into<PhysicalNode<K>> for Node<K, H, N> {
    fn into(self) -> PhysicalNode<K> {
        match self {
            Node::Internal(internal) => PhysicalNode::Internal(internal.into()),
            Node::Leaf(leaf) => PhysicalNode::Leaf(leaf.into()),
            Node::Null => PhysicalNode::Null,
        }
    }
}

// Derive is broken. See comment on SparseMerkleLeafNode<H, const N: usize>
// TODO: Add a proptest to enforce correctness.
impl<K: Clone, H, const N: usize> Clone for Node<K, H, N> {
    fn clone(&self) -> Self {
        match self {
            Self::Internal(arg0) => Self::Internal(arg0.clone()),
            Self::Leaf(arg0) => Self::Leaf(arg0.clone()),
            Self::Null => Self::Null,
        }
    }
}

impl<K, H, const N: usize> From<InternalNode<H, N>> for Node<K, H, N> {
    fn from(node: InternalNode<H, N>) -> Self {
        Node::Internal(node)
    }
}

impl<H, const N: usize> From<InternalNode<H, N>> for Children<N> {
    fn from(node: InternalNode<H, N>) -> Self {
        node.children
    }
}

impl<K, H, const N: usize> From<LeafNode<K, H, N>> for Node<K, H, N> {
    fn from(node: LeafNode<K, H, N>) -> Self {
        Node::Leaf(node)
    }
}

impl<K, H, const N: usize> Node<K, H, N>
where
    K: crate::Key,
    H: crate::TreeHash<N>,
{
    /// Creates the [`Internal`](Node::Internal) variant.
    #[cfg(any(test, feature = "fuzzing"))]
    pub fn new_internal(children: Children<N>) -> Self {
        Node::Internal(InternalNode::new(children))
    }

    /// Creates the [`Leaf`](Node::Leaf) variant.
    pub fn new_leaf(
        account_key: KeyHash<N>,
        value_hash: ValueHash<N>,
        value_index: (K, Version),
    ) -> Self {
        Node::Leaf(LeafNode::new(account_key, value_hash, value_index))
    }

    /// Returns `true` if the node is a leaf node.
    pub fn is_leaf(&self) -> bool {
        matches!(self, Node::Leaf(_))
    }

    /// Returns `NodeType`
    pub fn node_type(&self) -> NodeType {
        match self {
            // The returning value will be used to construct a `Child` of a internal node, while an
            // internal node will never have a child of Node::Null.
            Self::Leaf(_) => NodeType::Leaf,
            Self::Internal(n) => n.node_type(),
            Self::Null => NodeType::Null,
        }
    }

    /// Returns leaf count if known
    pub fn leaf_count(&self) -> usize {
        match self {
            Node::Leaf(_) => 1,
            Node::Internal(internal_node) => internal_node.leaf_count,
            Node::Null => 0,
        }
    }

    /// Serializes to bytes for physical storage.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = vec![];

        match self {
            Node::Internal(internal_node) => {
                out.push(NodeTag::Internal as u8);
                internal_node.serialize(&mut out)?;
                inc_internal_encoded_bytes_if_enabled(out.len())
            }
            Node::Leaf(leaf_node) => {
                out.push(NodeTag::Leaf as u8);
                leaf_node.serialize(&mut out)?;
                inc_leaf_encoded_bytes_if_enabled(out.len())
            }
            Node::Null => {
                out.push(NodeTag::Null as u8);
            }
        }
        Ok(out)
    }

    /// Computes the hash of nodes.
    pub fn hash(&self) -> HashOutput<N> {
        match self {
            Node::Internal(internal_node) => internal_node.hash(),
            Node::Leaf(leaf_node) => leaf_node.hash(),
            Node::Null => H::SPARSE_MERKLE_PLACEHOLDER_HASH,
        }
    }

    /// Recovers from serialized bytes in physical storage.
    pub fn decode(val: &[u8]) -> Result<Node<K, H, N>, CodecError> {
        if val.is_empty() {
            return Err(NodeDecodeError::EmptyInput.into());
        }
        let tag = val[0];
        let node_tag = NodeTag::from_u8(tag);
        match node_tag {
            Some(NodeTag::Internal) => Ok(Node::Internal(InternalNode::deserialize(&val[1..])?)),
            Some(NodeTag::Leaf) => Ok(Node::Leaf(LeafNode::deserialize(&val[1..])?)),
            Some(NodeTag::Null) => Ok(Node::Null),
            None => Err(NodeDecodeError::UnknownTag { unknown_tag: tag }.into()),
        }
    }
}

/// A type-erased [`Node`] - with no knowledge of the JMTs hash function or digest size.
/// Allows the creation of database abstractions without excessive generics.
#[derive(Debug, Eq, PartialEq, Clone)]
#[cfg_attr(
    any(test, feature = "borsh"),
    derive(::borsh::BorshDeserialize, ::borsh::BorshSerialize)
)]
pub enum PhysicalNode<K> {
    /// A wrapper of [`InternalNode`].
    Internal(PhysicalInternalNode),
    /// A wrapper of [`LeafNode`].
    Leaf(PhysicalLeafNode<K>),
    /// Represents empty tree only
    Null,
}

impl<K, H, const N: usize> TryFrom<PhysicalNode<K>> for Node<K, H, N> {
    type Error = CodecError;

    fn try_from(value: PhysicalNode<K>) -> Result<Self, Self::Error> {
        Ok(match value {
            PhysicalNode::Internal(n) => Self::Internal(n.try_into()?),
            PhysicalNode::Leaf(n) => Self::Leaf(n.try_into()?),
            PhysicalNode::Null => Self::Null,
        })
    }
}

/// Helper function to serialize version in a more efficient encoding.
/// We use a super simple encoding - the high bit is set if more bytes follow.
fn serialize_u64_varint(mut num: u64, binary: &mut Vec<u8>) {
    for _ in 0..8 {
        let low_bits = num as u8 & 0x7F;
        num >>= 7;
        let more = match num {
            0 => 0u8,
            _ => 0x80,
        };
        binary.push(low_bits | more);
        if more == 0 {
            return;
        }
    }
    // Last byte is encoded raw; this means there are no bad encodings.
    assert_ne!(num, 0);
    assert!(num <= 0xFF);
    binary.push(num as u8);
}

/// Helper function to deserialize versions from above encoding.
fn deserialize_u64_varint<T>(reader: &mut T) -> Result<u64, CodecError>
where
    T: Read,
{
    let mut num = 0u64;
    for i in 0..8 {
        let byte = reader.read_u8()?;
        num |= u64::from(byte & 0x7F) << (i * 7);
        if (byte & 0x80) == 0 {
            return Ok(num);
        }
    }
    // Last byte is encoded as is.
    let byte = reader.read_u8()?;
    num |= u64::from(byte) << 56;
    Ok(num)
}

pub struct MerkleTreeInternalNode<H, const N: usize> {
    left_child: HashOutput<N>,
    right_child: HashOutput<N>,
    hasher: std::marker::PhantomData<H>,
}

impl<H: TreeHash<N>, const N: usize> MerkleTreeInternalNode<H, N> {
    pub fn new(left_child: HashOutput<N>, right_child: HashOutput<N>) -> Self {
        Self {
            left_child,
            right_child,
            hasher: std::marker::PhantomData,
        }
    }

    pub fn hash(&self) -> HashOutput<N> {
        H::hasher()
            .update(self.left_child.as_ref())
            .update(self.right_child.as_ref())
            .finalize()
    }
}

#[cfg(any(test, feature = "fuzzing"))]
mod tests {
    use proptest::{prop_assert_eq, proptest};

    use crate::{
        node_type::{InternalNode, LeafNode, Node, NodeTag},
        test_helper::ValueBlob,
        test_utils::TestHash,
        KeyHash, ValueHash,
    };

    proptest! {
            #[test]
            fn test_clone_internal_node(node: InternalNode<TestHash, 32>) {
                let clone = node.clone();
                prop_assert_eq!(&clone, &node);

                let wrapped:Node<ValueBlob, TestHash, 32> = Node::Internal(node);
                let clone = wrapped.clone();
                prop_assert_eq!(wrapped, clone);
            }

            #[test]
            fn test_clone_leaf_node(h1: KeyHash<32>, h2: ValueHash<32>, value: ValueBlob, version: u64) {
                let node: LeafNode<ValueBlob, TestHash, 32>= LeafNode::new(h1, h2, (value, version));
                let clone = node.clone();
                prop_assert_eq!(&clone, &node);

                let wrapped:Node<ValueBlob, TestHash, 32> = Node::Leaf(node);
                let clone = wrapped.clone();
                prop_assert_eq!(wrapped, clone);
            }

            #[test]
            fn test_node_tag_roundtrip(tag: NodeTag) {
                prop_assert_eq!(NodeTag::from_u8(tag.to_u8()), Some(tag));
            }

    }
}
