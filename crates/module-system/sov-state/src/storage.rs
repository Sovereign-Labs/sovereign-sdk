//! Module storage definitions.

use core::fmt;
use std::fmt::Display;
use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use derivative::Derivative;
use jmt::KeyHash;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::reexports::digest::{typenum, Digest};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

use crate::bytes::Prefix;
use crate::codec::EncodeLike;
use crate::namespaces::{ProvableCompileTimeNamespace, ProvableNamespace};
use crate::{
    MerkleProofSpec, SparseMerkleProof, StateAccesses, StateItemDecoder, StorageRoot, Witness,
};

type ArcFormatFn =
    Arc<dyn (Fn(&[u8], &mut fmt::Formatter<'_>) -> fmt::Result) + Send + Sync + 'static>;

/// The key type suitable for use in [`Storage::get`] and other getter methods of
/// [`Storage`]. Cheaply-clonable.
#[derive(
    Derivative, Serialize, serde::Deserialize, BorshDeserialize, BorshSerialize, UniversalWallet,
)]
#[derivative(Clone, PartialEq, Eq, Debug, Hash, Ord)]
pub struct SlotKey {
    #[sov_wallet(hidden)]
    key: Arc<Vec<u8>>,
    #[borsh(skip)]
    #[serde(skip)]
    #[derivative(
        Debug = "ignore",
        PartialEq = "ignore",
        Hash = "ignore",
        Ord = "ignore"
    )]
    #[sov_wallet(skip)]
    display_fn: Option<ArcFormatFn>,
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for SlotKey {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        const MIN_LEN: usize = 10;
        const MAX_LEN: usize = 512;
        // Will include some non alhpanumeric characters, but that's fine.
        const ASCII_SELECTED: std::ops::RangeInclusive<u8> = b'0'..=b'z';

        let len = u.int_in_range(MIN_LEN..=MAX_LEN)?;

        let key: arbitrary::Result<Vec<u8>> =
            std::iter::repeat_with(|| u.int_in_range(ASCII_SELECTED.clone()))
                .take(len)
                .collect();

        Ok(SlotKey::from(key?))
    }
}

// Manually implement PartialOrd to satisfy clippy
impl PartialOrd for SlotKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl From<Vec<u8>> for SlotKey {
    fn from(key: Vec<u8>) -> Self {
        Self {
            key: Arc::new(key),
            display_fn: None,
        }
    }
}

impl SlotKey {
    /// Returns a new [`Arc`] reference to the bytes of this key.
    pub fn key(&self) -> Arc<Vec<u8>> {
        self.key.clone()
    }

    /// Returns a new [`Arc`] reference to the bytes of this key.
    pub fn key_ref(&self) -> &Vec<u8> {
        self.key.as_ref()
    }

    /// Returns the size of the key.
    pub fn size(&self) -> usize {
        self.key.len()
    }
}

impl AsRef<Vec<u8>> for SlotKey {
    fn as_ref(&self) -> &Vec<u8> {
        &self.key
    }
}

impl fmt::Display for SlotKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(display_fn) = &self.display_fn {
            display_fn(self.key.as_ref(), f)
        } else {
            write!(f, "{}", String::from_utf8_lossy(self.key().as_ref()))
        }
    }
}

impl SlotKey {
    /// Creates a new [`SlotKey`] that combines a prefix and a key.
    pub fn new<K, Q, KC>(prefix: &Prefix, key: &Q, codec: &KC) -> Self
    where
        KC: EncodeLike<Q, K> + StateItemDecoder<K> + 'static,
        K: fmt::Display,
        Q: ?Sized,
    {
        let encoded_key = codec.encode_like(key);

        let mut full_key = Vec::<u8>::with_capacity(prefix.len().saturating_add(encoded_key.len()));
        full_key.extend(prefix.as_ref());
        full_key.extend(&encoded_key);
        let prefix_len = prefix.len();
        let codec = codec.clone();
        let display_fn: Option<ArcFormatFn> = Some(Arc::new(
            move |key_bytes: &[u8], formatter: &mut fmt::Formatter<'_>| {
                if key_bytes.len() < prefix_len {
                    return Err(std::fmt::Error);
                }
                let prefix = &key_bytes[..prefix_len];
                let key = &key_bytes[prefix_len..];
                let key = KC::try_decode(&codec, key).map_err(|_| std::fmt::Error)?;
                let prefix_str = std::str::from_utf8(prefix).map_err(|_e| std::fmt::Error)?;
                write!(formatter, "{prefix_str}{key}")
            },
        ));
        Self {
            key: Arc::new(full_key),
            display_fn,
        }
    }

    /// Used only in tests.
    /// Builds a storage key from a byte slice
    pub fn from_slice(key: &[u8]) -> Self {
        Self {
            key: Arc::new(key.to_vec()),
            display_fn: None,
        }
    }

    /// Creates a new [`SlotKey`] from a prefix.
    pub fn singleton(prefix: &Prefix) -> Self {
        Self {
            key: Arc::new(prefix.as_ref().to_vec()),
            display_fn: Some(Arc::new(
                move |key_bytes: &[u8], formatter: &mut fmt::Formatter<'_>| {
                    let prefix_str =
                        std::str::from_utf8(key_bytes).map_err(|_e| std::fmt::Error)?;
                    formatter.write_str(prefix_str)
                },
            )),
        }
    }
}

// We return `Vec<u8>` here to be compatible with the `JMT::put_value_set_with_proof` method.
fn val_hash_and_size_inner(val_hash: [u8; 32], size: u32) -> Vec<u8> {
    let mut val_hash_and_size = Vec::with_capacity(40);
    let size_bytes = size.to_le_bytes();
    val_hash_and_size.extend_from_slice(&val_hash);
    val_hash_and_size.extend_from_slice(&size_bytes);
    val_hash_and_size
}

/// A serialized value suitable for storing. Internally uses an [`Arc<Vec<u8>>`]
/// for cheap cloning.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Default,
    Serialize,
    serde::Deserialize,
    BorshDeserialize,
    BorshSerialize,
    UniversalWallet,
)]
pub struct SlotValue {
    #[sov_wallet(hidden)]
    value: Arc<Vec<u8>>,
}

impl From<Vec<u8>> for SlotValue {
    fn from(value: Vec<u8>) -> Self {
        Self {
            value: Arc::new(value),
        }
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for SlotValue {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        const MIN_LEN: usize = 32;
        const MAX_LEN: usize = 1024;
        let len = u.int_in_range(MIN_LEN..=MAX_LEN)?;
        let value = u.bytes(len)?.to_vec();
        Ok(SlotValue::from(value))
    }
}

impl SlotValue {
    /// Create a new storage value by serializing the input with the given codec.
    pub fn new<V, Vq, VC>(value: &Vq, codec: &VC) -> Self
    where
        Vq: ?Sized,
        VC: EncodeLike<Vq, V>,
    {
        let encoded_value = codec.encode_like(value);
        Self {
            value: Arc::new(encoded_value),
        }
    }

    /// Get a debug string for an optional value suitable for logging.
    pub fn debug_show(value: Option<&Self>) -> String {
        match value {
            Some(v) => {
                if v.value.len() > 128 {
                    format!("{:?}...", &v.value.as_ref()[..128])
                } else {
                    format!("{:?}", v.value.as_ref())
                }
            }
            None => "None".to_string(),
        }
    }

    /// Get the bytes of this value.
    pub fn value(&self) -> &[u8] {
        &self.value
    }

    /// The size of the `SlotValue` in bytes.
    /// Panics if size can't be represented as u32.
    pub fn size(&self) -> u32 {
        self.value
            .len()
            .try_into()
            .expect("Overflow: Unable to cast usize to u32.")
    }

    /// Combines the value hash with its size.
    pub(crate) fn combine_val_hash_and_size<H: Digest<OutputSize = typenum::U32>>(
        &self,
    ) -> Vec<u8> {
        let val_hash: [u8; 32] = H::digest(self.value.as_ref()).into();
        val_hash_and_size_inner(val_hash, self.size())
    }
}

#[derive(
    Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize, BorshDeserialize, BorshSerialize,
)]
pub(crate) enum ReadType {
    // `get_size` didn't return the full value.
    GetSizeValueNotFetched,
    // `get_size` returned the full value.
    GetSizeValueFetched(SlotValue),
    // Read operation.
    Read(SlotValue),
}

/// Data that is saved in the `Read` cache.
#[derive(
    Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize, BorshDeserialize, BorshSerialize,
)]
pub struct NodeLeafAndMaybeValue {
    pub(crate) leaf: NodeLeaf,
    pub(crate) value: ReadType,
}

/// Size and hash of a value saved in the state.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Default,
    Serialize,
    serde::Deserialize,
    BorshDeserialize,
    BorshSerialize,
)]
pub struct NodeLeaf {
    /// The size of the value.
    pub(crate) size: u32,
    /// The hash of the value.
    pub(crate) val_hash: [u8; 32],
}

impl NodeLeaf {
    // TODO: make it `pub(crate)` again after nomt integration is completed.
    #[allow(missing_docs)]
    pub fn make_leaf<H: Digest<OutputSize = typenum::U32>>(value: &SlotValue) -> NodeLeaf {
        let size = value.size();
        let val_hash: [u8; 32] = H::digest(value.value()).into();
        NodeLeaf { size, val_hash }
    }

    /// Combines the value hash with its size.
    pub(crate) fn combine_val_hash_and_size(&self) -> Vec<u8> {
        val_hash_and_size_inner(self.val_hash, self.size)
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    serde::Deserialize,
    BorshDeserialize,
    BorshSerialize,
    UniversalWallet,
)]
/// A proof that a particular storage key has a particular value, or is absent.
// Note: This type intentionally does not derive `UniversalWallet` because the slotkey and slotvalue
// can't be displayed meaningfully without additional context
pub struct StorageProof<P> {
    /// The key which is proven
    pub key: SlotKey,
    /// The value, if any, which is proven
    pub value: Option<SlotValue>,
    /// The cryptographic proof
    #[sov_wallet(hidden)]
    pub proof: P,
    /// The namespace of the key.
    pub namespace: ProvableNamespace,
}

/// A trait implemented by state updates that can be committed to the database.
pub trait StateUpdate {
    /// Adds a non-provable ("accessory") state change to the
    /// state update after the rest of the update is finalized.
    fn add_accessory_item(&mut self, key: SlotKey, value: Option<SlotValue>);

    /// Adds a collection of non-provable ("accessory") state changes to the
    /// state update after the rest of the update is finalized.
    fn add_accessory_items(&mut self, items: Vec<(SlotKey, Option<SlotValue>)>) {
        for (key, value) in items {
            self.add_accessory_item(key, value);
        }
    }

    /// Returns an iterator over the accessory items modified by this state update.
    fn get_accessory_items(&self) -> impl Iterator<Item = &(SlotKey, Option<SlotValue>)>;
}

impl StateUpdate for () {
    fn add_accessory_item(&mut self, _key: SlotKey, _value: Option<SlotValue>) {
        // Silently discard the input. This is safe, since the accessory state
        // is *not* consensus critical. This implementation is intended to be used
        // in the zk context only. In the native context, a real implementation SHOULD
        // be used instead.
    }

    fn get_accessory_items(&self) -> impl Iterator<Item = &(SlotKey, Option<SlotValue>)> {
        std::iter::empty()
    }
}

/// A trait that represents the root hash of a state tree.
pub trait StateRoot:
    Serialize
    + DeserializeOwned
    + fmt::Debug
    + Display
    + Clone
    + BorshSerialize
    + BorshDeserialize
    + Eq
    + Send
    + Sync
    + AsRef<[u8]>
{
    /// Gets the global root hash of the storage. Ie, the root hash of the entire tree for all namespaces.
    /// We always require a one-way conversion from the state root to a 32-byte array. This can be
    /// implemented by hashing the state root even if the root itself is not 32 bytes.
    fn global_root(&self) -> [u8; 32];

    /// Gets the root hash of a specific namespace
    fn namespace_root(&self, namespace: ProvableNamespace) -> [u8; 32];

    /// Builds a storage root from underlying namespace roots.
    fn from_namespace_roots(user_root: [u8; 32], kernel_root: [u8; 32]) -> Self;
}

/// An interface for retrieving values from the storage and producing change set of new write operations.
pub trait Storage: Clone + core::fmt::Debug {
    /// Hasher
    type Hasher: Digest<OutputSize = typenum::U32> + Send + Sync;

    /// The witness type for this storage instance.
    type Witness: Witness + Send + Sync + core::fmt::Debug;

    /// A cryptographic proof that a particular key has a particular value, or is absent.
    type Proof: Serialize
        + DeserializeOwned
        + fmt::Debug
        + Clone
        + BorshSerialize
        + BorshDeserialize
        + Send
        + Sync
        + PartialEq
        + Eq;

    /// A cryptographic commitment to the contents of this storage.
    type Root: StateRoot;

    /// State update that will be committed to the database.
    type StateUpdate: StateUpdate;

    /// Collections of all the writes that have been made on top of this instance of the storage;
    type ChangeSet: Send + Sync;

    /// The root hash of storage *before* genesis, when it's completely empty.
    const PRE_GENESIS_ROOT: Self::Root;

    /// Puts the value in the witness.
    fn put_in_witness(&self, value: Option<SlotValue>, witness: &Self::Witness);

    /// Get the node leaf. This method does not need to load the full value into memory.
    fn get_leaf<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue>;

    /// Returns the value corresponding to the key or None if key is absent.
    fn get<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<SlotValue>;

    /// Returns the value corresponding to the key or None if key is absent.
    fn get_accessory(&self, _key: &SlotKey, _version: Option<SlotNumber>) -> Option<SlotValue>;

    /// Calculates new state root but does not commit any changes to the database.
    fn compute_state_update(
        &self,
        state_accesses: StateAccesses,
        witness: &Self::Witness,
        prev_state_root: Self::Root,
    ) -> anyhow::Result<(Self::Root, Self::StateUpdate)>;

    /// Materializes changes from given [`Self::StateUpdate`] into [`Self::ChangeSet`].
    fn materialize_changes(self, state_update: Self::StateUpdate) -> Self::ChangeSet;

    /// Opens a storage access proof and validates it against a state root.
    /// It returns a result with the opened leaf (key, value) pair in case of success.
    fn open_proof(
        state_root: Self::Root,
        proof: StorageProof<Self::Proof>,
    ) -> anyhow::Result<(SlotKey, Option<SlotValue>)>;
}

/// Used only in tests.
impl From<&str> for SlotValue {
    fn from(value: &str) -> Self {
        Self {
            value: Arc::new(value.as_bytes().to_vec()),
        }
    }
}

#[cfg(feature = "native")]
/// A [`Storage`] that is suitable for use in native execution environments
/// (outside of the zkVM).
pub trait NativeStorage: Storage {
    /// Gets the latest version available in the current instance storage.
    fn latest_version(&self) -> SlotNumber;

    /// Gets the latest version that can be committed from any other instance of the storage.
    fn latest_version_unbound(&self) -> SlotNumber;

    /// Returns the value corresponding to the key or None if key is absent.
    fn get_historical<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
        witness: &Self::Witness,
    ) -> Option<SlotValue>;

    /// Get the node leaf. This method does not need to load the full value into memory.
    fn get_leaf_historical<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
        witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue>;

    /// Returns the value corresponding to the key or None if the key is absent and a proof to
    /// get the value.
    /// Returns an error if storage is empty or the passed version is not yet available.
    fn get_with_proof<N: ProvableCompileTimeNamespace>(
        &self,
        key: SlotKey,
        slot_number: Option<SlotNumber>,
    ) -> anyhow::Result<StorageProof<Self::Proof>>;

    /// Get the *global* root hash of the tree at the requested version.
    /// Returns an error if storage is empty or the requests version is not yet available.
    fn get_root_hash(&self, version: SlotNumber) -> anyhow::Result<Self::Root>;

    /// Get the *global* root hash of the tree at the requested version.
    /// Requested version won't be checked against latest version of this instance of the storage.
    fn get_root_hash_unbound(&self, version: SlotNumber) -> anyhow::Result<Self::Root>;

    /// Get a root hash at the latest version
    fn get_latest_root_hash(&self) -> anyhow::Result<Self::Root> {
        self.get_root_hash(self.latest_version())
    }
}

pub(crate) fn open_merkle_proof<S: MerkleProofSpec>(
    state_root: StorageRoot<S>,
    state_proof: StorageProof<SparseMerkleProof<S::Hasher>>,
) -> anyhow::Result<(SlotKey, Option<SlotValue>)> {
    let StorageProof {
        key,
        value,
        proof,
        namespace,
    } = state_proof;
    let key_hash = KeyHash::with::<S::Hasher>(key.as_ref());

    // The proof leaves contain hash(combine(val_hash, val_len)).
    // The outer hashing is handled by the verify method, so we need to pass combine(val_hash, val_len).
    let val_hash_and_size = value
        .as_ref()
        .map(SlotValue::combine_val_hash_and_size::<S::Hasher>);

    proof.inner().verify(
        // We need to verify the proof against the correct root hash.
        // Hence we match the key against its namespace
        jmt::RootHash(state_root.namespace_root(namespace)),
        key_hash,
        val_hash_and_size,
    )?;

    Ok((key, value))
}
