#[cfg(any(test, feature = "fuzzing"))]
use proptest_derive::Arbitrary;
use serde::{Deserialize, Serialize};

use crate::{
    errors::ProofError,
    hash::{CryptoHasher, HashOutput, TreeHash},
    node_type::MerkleTreeInternalNode,
    KeyHash, ValueHash,
};

/// A proof that can be used to authenticate an element in a Sparse Merkle Tree given trusted root
/// hash. For example, `TransactionInfoToAccountProof` can be constructed on top of this structure.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SparseMerkleProof<H, const N: usize> {
    /// This proof can be used to authenticate whether a given leaf exists in the tree or not.
    ///     - If this is `Some(leaf_node)`
    ///         - If `leaf_node.key` equals requested key, this is an inclusion proof and
    ///           `leaf_node.value_hash` equals the hash of the corresponding account blob.
    ///         - Otherwise this is a non-inclusion proof. `leaf_node.key` is the only key
    ///           that exists in the subtree and `leaf_node.value_hash` equals the hash of the
    ///           corresponding account blob.
    ///     - If this is `None`, this is also a non-inclusion proof which indicates the subtree is
    ///       empty.
    leaf: Option<SparseMerkleLeafNode<H, N>>,

    /// All siblings in this proof, including the default ones. Siblings are ordered from the bottom
    /// level to the root level.
    siblings: Vec<HashOutput<N>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeInProof<H, const N: usize> {
    Leaf(SparseMerkleLeafNode<H, N>),
    Other(HashOutput<N>),
}

impl<H, const N: usize> From<HashOutput<N>> for NodeInProof<H, N> {
    fn from(hash: HashOutput<N>) -> Self {
        Self::Other(hash)
    }
}

impl<H, const N: usize> From<SparseMerkleLeafNode<H, N>> for NodeInProof<H, N> {
    fn from(leaf: SparseMerkleLeafNode<H, N>) -> Self {
        Self::Leaf(leaf)
    }
}

impl<H: TreeHash<N>, const N: usize> NodeInProof<H, N> {
    pub fn hash(&self) -> HashOutput<N> {
        match self {
            Self::Leaf(leaf) => leaf.hash(),
            Self::Other(hash) => *hash,
        }
    }
}

/// A more detailed version of `SparseMerkleProof` with the only difference that all the leaf
/// siblings are explicitly set as `SparseMerkleLeafNode` instead of its hash value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SparseMerkleProofExt<H, const N: usize> {
    leaf: Option<SparseMerkleLeafNode<H, N>>,
    /// All siblings in this proof, including the default ones. Siblings are ordered from the bottom
    /// level to the root level.
    siblings: Vec<NodeInProof<H, N>>,
}

impl<H: TreeHash<N>, const N: usize> SparseMerkleProofExt<H, N> {
    /// Constructs a new `SparseMerkleProofExt` using leaf and a list of sibling nodes.
    pub fn new(leaf: Option<SparseMerkleLeafNode<H, N>>, siblings: Vec<NodeInProof<H, N>>) -> Self {
        Self { leaf, siblings }
    }

    /// Returns the leaf node in this proof.
    pub fn leaf(&self) -> Option<SparseMerkleLeafNode<H, N>> {
        self.leaf.clone()
    }

    /// Returns the list of siblings in this proof.
    pub fn siblings(&self) -> &[NodeInProof<H, N>] {
        &self.siblings
    }

    pub fn verify(
        &self,
        expected_root_hash: HashOutput<N>,
        element_key: KeyHash<N>,
        element_value: Option<&[u8]>,
    ) -> Result<(), ProofError<N>> {
        SparseMerkleProof::from(self).verify(expected_root_hash, element_key, element_value)
    }

    pub fn verify_by_hash(
        &self,
        expected_root_hash: HashOutput<N>,
        element_key: KeyHash<N>,
        element_hash: Option<ValueHash<N>>,
    ) -> Result<(), ProofError<N>> {
        SparseMerkleProof::from(self).verify_by_hash(expected_root_hash, element_key, element_hash)
    }
}

impl<H: TreeHash<N>, const N: usize> From<SparseMerkleProofExt<H, N>> for SparseMerkleProof<H, N> {
    fn from(proof_ext: SparseMerkleProofExt<H, N>) -> Self {
        Self::new(
            proof_ext.leaf,
            proof_ext
                .siblings
                .into_iter()
                .map(|node| node.hash())
                .collect(),
        )
    }
}

impl<H: TreeHash<N>, const N: usize> From<&SparseMerkleProofExt<H, N>> for SparseMerkleProof<H, N> {
    fn from(proof_ext: &SparseMerkleProofExt<H, N>) -> Self {
        let leaf = proof_ext.leaf.clone();
        Self::new(
            leaf,
            proof_ext.siblings.iter().map(|node| node.hash()).collect(),
        )
    }
}

impl<H: TreeHash<N>, const N: usize> SparseMerkleProof<H, N> {
    /// Constructs a new `SparseMerkleProof` using leaf and a list of siblings.
    pub fn new(leaf: Option<SparseMerkleLeafNode<H, N>>, siblings: Vec<HashOutput<N>>) -> Self {
        SparseMerkleProof { leaf, siblings }
    }

    /// Returns the leaf node in this proof.
    pub fn leaf(&self) -> &Option<SparseMerkleLeafNode<H, N>> {
        &self.leaf
    }

    /// Returns the list of siblings in this proof.
    pub fn siblings(&self) -> &[HashOutput<N>] {
        &self.siblings
    }

    pub fn verify(
        &self,
        expected_root_hash: HashOutput<N>,
        element_key: KeyHash<N>,
        element_value: Option<&[u8]>,
    ) -> Result<(), ProofError<N>> {
        self.verify_by_hash(
            expected_root_hash,
            element_key,
            element_value.map(|v| ValueHash(H::hash(v))),
        )
    }

    /// If `element_hash` is present, verifies an element whose key is `element_key` and value is
    /// authenticated by `element_hash` exists in the Sparse Merkle Tree using the provided proof.
    /// Otherwise verifies the proof is a valid non-inclusion proof that shows this key doesn't
    /// exist in the tree.
    pub fn verify_by_hash(
        &self,
        expected_root_hash: HashOutput<N>,
        element_key: KeyHash<N>,
        element_hash: Option<ValueHash<N>>,
    ) -> Result<(), ProofError<N>> {
        if self.siblings.len() > HashOutput::<N>::LENGTH_IN_BITS {
            return Err(ProofError::TooManySiblings {
                got: self.siblings.len(),
            });
        }

        match (element_hash, &self.leaf) {
            (Some(hash), Some(leaf)) => {
                // This is an inclusion proof, so the key and value hash provided in the proof
                // should match element_key and element_value_hash. `siblings` should prove the
                // route from the leaf node to the root.
                if element_key != leaf.key {
                    return Err(ProofError::KeyMismatch {
                        expected: element_key,
                        got: leaf.key,
                    });
                }

                if hash != leaf.value_hash {
                    return Err(ProofError::ValueMismatch {
                        key: leaf.key,
                        expected: hash,
                        got: leaf.value_hash,
                    });
                }
            }
            (Some(hash), None) => {
                return Err(ProofError::ExpectedInclusionProof { value_hash: hash });
            }
            (None, Some(leaf)) => {
                // This is a non-inclusion proof. The proof intends to show that if a leaf node
                // representing `element_key` is inserted, it will break a currently existing leaf
                // node represented by `proof_key` into a branch. `siblings` should prove the
                // route from that leaf node to the root.
                if element_key == leaf.key {
                    return Err(ProofError::ExpectedNonInclusionProof {
                        leaf_key: element_key,
                    });
                }
                if element_key.common_prefix_bits_len(&leaf.key) < self.siblings.len() {
                    return Err(ProofError::InvalidNonInclusionProof {
                        key_in_proof: leaf.key,
                        key_to_verify: element_key,
                    });
                };
            }
            (None, None) => {
                // This is a non-inclusion proof. The proof intends to show that if a leaf node
                // representing `element_key` is inserted, it will show up at a currently empty
                // position. `sibling` should prove the route from this empty position to the root.
            }
        }

        let current_hash = self
            .leaf
            .as_ref()
            .map_or(H::SPARSE_MERKLE_PLACEHOLDER_HASH, |leaf| leaf.hash());
        let actual_root_hash = self
            .siblings
            .iter()
            .zip(
                element_key
                    .iter_bits()
                    .rev()
                    .skip(HashOutput::<N>::LENGTH_IN_BITS - self.siblings.len()),
            )
            .fold(current_hash, |hash, (sibling_hash, bit)| {
                if bit {
                    MerkleTreeInternalNode::<H, N>::new(*sibling_hash, hash).hash()
                } else {
                    MerkleTreeInternalNode::<H, N>::new(hash, *sibling_hash).hash()
                }
            });
        if actual_root_hash != expected_root_hash {
            return Err(ProofError::IncorrectRoot {
                expected: expected_root_hash,
                got: actual_root_hash,
            });
        }
        Ok(())
    }
}

/// Note: this is not a range proof in the sense that a range of nodes is verified!
/// Instead, it verifies the entire left part of the tree up to a known rightmost node.
/// See the description below.
///
/// A proof that can be used to authenticate a range of consecutive leaves, from the leftmost leaf to
/// the rightmost known one, in a sparse Merkle tree. For example, given the following sparse Merkle tree:
///
/// ```text
///                   root
///                  /     \
///                 /       \
///                /         \
///               o           o
///              / \         / \
///             a   o       o   h
///                / \     / \
///               o   d   e   X
///              / \         / \
///             b   c       f   g
/// ```
///
/// if the proof wants show that `[a, b, c, d, e]` exists in the tree, it would need the siblings
/// `X` and `h` on the right.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SparseMerkleRangeProof<H, const N: usize> {
    /// The vector of siblings on the right of the path from root to last leaf. The ones near the
    /// bottom are at the beginning of the vector. In the above example, it's `[X, h]`.
    right_siblings: Vec<HashOutput<N>>,
    phantom_hasher: std::marker::PhantomData<H>,
}

impl<H: TreeHash<N>, const N: usize> SparseMerkleRangeProof<H, N> {
    /// Constructs a new `SparseMerkleRangeProof`.
    pub fn new(right_siblings: Vec<HashOutput<N>>) -> Self {
        Self {
            right_siblings,
            phantom_hasher: std::marker::PhantomData,
        }
    }

    /// Returns the right siblings.
    pub fn right_siblings(&self) -> &[HashOutput<N>] {
        &self.right_siblings
    }

    /// Verifies that the rightmost known leaf exists in the tree and that the resulting
    /// root hash matches the expected root hash.
    pub fn verify(
        &self,
        expected_root_hash: HashOutput<N>,
        rightmost_known_leaf: SparseMerkleLeafNode<H, N>,
        left_siblings: Vec<HashOutput<N>>,
    ) -> Result<(), ProofError<N>> {
        let num_siblings = left_siblings.len() + self.right_siblings.len();
        let mut left_sibling_iter = left_siblings.iter();
        let mut right_sibling_iter = self.right_siblings().iter();

        let mut current_hash = rightmost_known_leaf.hash();
        for bit in rightmost_known_leaf
            .key()
            .iter_bits()
            .rev()
            .skip(HashOutput::<N>::LENGTH_IN_BITS - num_siblings)
        {
            let (left_hash, right_hash) = if bit {
                (
                    *left_sibling_iter
                        .next()
                        .ok_or(ProofError::MissingLeftSibling {
                            needed: rightmost_known_leaf
                                .key()
                                .iter_bits()
                                .rev()
                                .skip(HashOutput::<N>::LENGTH_IN_BITS - num_siblings)
                                .filter(|b| *b)
                                .count(),
                            got: left_siblings.clone(),
                        })?,
                    current_hash,
                )
            } else {
                (
                    current_hash,
                    *right_sibling_iter
                        .next()
                        .ok_or(ProofError::MissingLeftSibling {
                            needed: rightmost_known_leaf
                                .key()
                                .iter_bits()
                                .rev()
                                .skip(HashOutput::<N>::LENGTH_IN_BITS - num_siblings)
                                .filter(|b| !b)
                                .count(),
                            got: self.right_siblings.clone(),
                        })?,
                )
            };
            current_hash = MerkleTreeInternalNode::<H, N>::new(left_hash, right_hash).hash();
        }

        if current_hash != expected_root_hash {
            return Err(ProofError::IncorrectRoot {
                expected: expected_root_hash,
                got: current_hash,
            });
        }
        Ok(())
    }
}

#[derive(Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
pub struct SparseMerkleLeafNode<H, const N: usize> {
    key: KeyHash<N>,
    value_hash: ValueHash<N>,
    phantom_hasher: std::marker::PhantomData<H>,
}
// Implement clone manually since Derive is broken.
// TODO: root cause. Maybe a compiler bug?
// It may be related to https://github.com/rust-lang/rust/issues/26925
//
// Steps to reproduce:
//  1. Delete this manual impl
//  2. Add #[derive(Clone)] annotation to SparseMerkleLeafNode<H, const N: usize>
//  3. Profit
impl<H, const N: usize> Clone for SparseMerkleLeafNode<H, N> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            value_hash: self.value_hash.clone(),
            phantom_hasher: self.phantom_hasher.clone(),
        }
    }
}

impl<H: TreeHash<N>, const N: usize> SparseMerkleLeafNode<H, N> {
    pub fn new(key: KeyHash<N>, value_hash: ValueHash<N>) -> Self {
        SparseMerkleLeafNode {
            key,
            value_hash,
            phantom_hasher: std::marker::PhantomData,
        }
    }

    pub fn key(&self) -> KeyHash<N> {
        self.key
    }

    pub fn value_hash(&self) -> ValueHash<N> {
        self.value_hash
    }

    pub fn hash(&self) -> HashOutput<N> {
        H::hasher()
            .update(self.key.0.as_ref())
            .update(self.value_hash.0.as_ref())
            .finalize()
    }
}

#[cfg(any(test, feature = "fuzzing"))]
mod tests {
    use proptest::proptest;

    use crate::test_utils::TestHash;

    use super::SparseMerkleLeafNode;

    proptest! {
    #[test]
    fn test_clone_sparse_merkle_leaf_node(node: SparseMerkleLeafNode<TestHash, 32>) {
        let clone = node.clone();
        assert_eq!(clone, node);
    }}
}
