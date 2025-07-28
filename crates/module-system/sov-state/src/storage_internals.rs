//! Defines the data structures needed by both the zk-storage and the prover storage.

use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use derivative::Derivative;
use jmt::SimpleHasher;
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;
use sov_rollup_interface::reexports::digest::Digest;
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

use crate::{MerkleProofSpec, ProvableNamespace, StateRoot};
/// Combined root hash of the user and kernel namespaces. The user root hash is the first 32 bytes, whereas the
/// kernel root hash is the last 32 bytes.
/// We need to store both the user and the kernel root hashes to be able to check zk-proofs against the
/// correct hash.
/// We use the generic `S: MerkleProofSpec` to specify the hash function used to compute the global root hash.
/// The global root hash is computed by hashing the user hash and the kernel hash together.
#[derive(
    Derivative,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    derive_more::Display,
    derive_more::Debug,
    UniversalWallet,
)]
#[derivative(
    Eq(bound = "S: MerkleProofSpec"),
    PartialEq(bound = "S: MerkleProofSpec"),
    Copy(bound = "S: MerkleProofSpec")
)]
#[display("{}", hex::encode(self.root_hashes))]
#[debug("StorageRoot {{ root_hashes: {} }}", hex::encode(self.root_hashes))]
pub struct StorageRoot<S: MerkleProofSpec> {
    #[serde(with = "BigArray")]
    root_hashes: [u8; 64],
    #[derivative(Debug = "ignore")]
    phantom: PhantomData<S>,
}

impl<S: MerkleProofSpec> AsRef<[u8]> for StorageRoot<S> {
    fn as_ref(&self) -> &[u8] {
        &self.root_hashes
    }
}

impl<S: MerkleProofSpec> Clone for StorageRoot<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: MerkleProofSpec> StateRoot for StorageRoot<S> {
    fn global_root(&self) -> [u8; 32] {
        self.root_hash()
    }

    fn from_namespace_roots(user_root: [u8; 32], kernel_root: [u8; 32]) -> Self {
        Self::new(user_root, kernel_root)
    }

    fn namespace_root(&self, namespace: ProvableNamespace) -> [u8; 32] {
        let mut output = [0u8; 32];

        match namespace {
            ProvableNamespace::Kernel => {
                output.copy_from_slice(&self.root_hashes[32..]);
            }
            ProvableNamespace::User => {
                output.copy_from_slice(&self.root_hashes[..32]);
            }
        }

        output
    }
}

impl<S: MerkleProofSpec> StorageRoot<S> {
    /// Creates a new `[ProverStorageRoot]` instance from specified root hashes.
    /// Concretely this method builds the prover root hash by concatenating the user and
    /// the kernel root hashes.
    pub const fn new(user_hash: [u8; 32], kernel_hash: [u8; 32]) -> Self {
        // Concatenate the user and kernel root hashes
        let mut root_hashes = [0u8; 64];
        let mut i = 0;
        // We don't have access to `for` loops or `copy_from_slice` in const fns - so we use a while loop even though it's a bit awkward
        while i < 32 {
            root_hashes[i] = user_hash[i];
            i += 1;
        }
        while i < 64 {
            root_hashes[i] = kernel_hash[i - 32];
            i += 1;
        }

        Self {
            root_hashes,
            phantom: PhantomData,
        }
    }

    /// Returns the global root hash of the prover storage.
    pub fn root_hash(&self) -> [u8; 32] {
        let mut hasher = <S::Hasher as Digest>::new();
        Digest::update(&mut hasher, self.namespace_root(ProvableNamespace::User));
        Digest::update(&mut hasher, self.namespace_root(ProvableNamespace::Kernel));
        Digest::finalize(hasher).into()
    }
}

/// A storage proof that is used to verify the existence of a key in the storage.
#[derive(Derivative, Serialize, Deserialize, BorshDeserialize, BorshSerialize, UniversalWallet)]
#[derivative(
    PartialEq(bound = "H: SimpleHasher"),
    Eq(bound = "H: SimpleHasher"),
    Clone(bound = "H: SimpleHasher"),
    Debug(bound = "H: SimpleHasher")
)]
pub struct SparseMerkleProof<H: SimpleHasher>(
    #[serde(bound(serialize = "", deserialize = ""))]
    #[borsh(bound(serialize = "", deserialize = ""))]
    #[sov_wallet(as_ty = "wallet_placeholders::MerkleDisplayPlaceholder")]
    jmt::proof::SparseMerkleProof<H>,
);

// The types in this module aren't actually dead code, they are used as placeholders in the wallet
// However, since they only appear in the Schema (which isn't Rust code), Rustc doesn't know that.
#[allow(dead_code)]
mod wallet_placeholders {
    use sov_rollup_interface::sov_universal_wallet::UniversalWallet;
    #[derive(UniversalWallet)]
    pub struct MerkleDisplayPlaceholder {
        leaf: Option<SparseMerkleLeafNodePlacholder>,
        siblings: Vec<SparseMerkleNodePlaceholder>,
    }

    #[derive(UniversalWallet)]
    struct SparseMerkleInternalNodePlaceholder {
        left_child: [u8; 32],
        right_child: [u8; 32],
    }

    #[derive(UniversalWallet)]
    enum SparseMerkleNodePlaceholder {
        // The default sparse node
        Null,
        // The internal sparse merkle tree node
        Internal(SparseMerkleInternalNodePlaceholder),
        // The leaf sparse merkle tree node
        Leaf(SparseMerkleLeafNodePlacholder),
    }

    #[derive(UniversalWallet)]
    pub struct SparseMerkleLeafNodePlacholder {
        key_hash: [u8; 32],
        value_hash: [u8; 32],
    }
}

impl<H: SimpleHasher> SparseMerkleProof<H> {
    /// Returns the underlying proof.
    pub fn inner(&self) -> &jmt::proof::SparseMerkleProof<H> {
        &self.0
    }
}

impl<H: SimpleHasher> From<jmt::proof::SparseMerkleProof<H>> for SparseMerkleProof<H> {
    fn from(proof: jmt::proof::SparseMerkleProof<H>) -> Self {
        SparseMerkleProof(proof)
    }
}
