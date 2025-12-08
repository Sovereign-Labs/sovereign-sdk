//! Module storage definitions.

use core::fmt;
use std::fmt::Display;

use borsh::{BorshDeserialize, BorshSerialize};
use jmt::KeyHash;
use serde::de::DeserializeOwned;
use serde::{Serialize};
#[cfg(feature = "native")]
use sov_rollup_interface::common::{RollupHeight, SlotNumber};
use sov_rollup_interface::reexports::digest::{typenum, Digest};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

use crate::codec::EncodeLike;
use crate::namespaces::{ProvableCompileTimeNamespace, ProvableNamespace};
use crate::pinned_cache::PinnedCache;
#[cfg(feature = "native")]
use crate::sequencer_state::MaybePresentValue;
#[cfg(feature = "native")]
use crate::{CompileTimeNamespace, Namespace};
use crate::{
    MerkleProofSpec, SparseMerkleProof, StateAccesses, StateItemDecoder, StorageRoot, Witness,
};

pub use sov_db::schema::types::slot_key::Prefix;
pub use sov_db::schema::types::slot_key::SlotKey;
pub use sov_db::schema::types::slot_key::SlotKeyBuilder;
pub use sov_db::schema::types::slot_key::SlotValue;
pub use sov_db::schema::types::slot_key::val_hash_and_size_inner;

/// A trait for creating a new key from a prefix and a key.
pub trait SlotKeyFromCodec {
    /// Creates a new [`SlotKey`] that combines a prefix and a key.
    fn new<K, Q, KC>(prefix: &Prefix, key: &Q, codec: &KC) -> Self
    where
        KC: EncodeLike<Q, K> + StateItemDecoder<K> + 'static,
        K: fmt::Display,
        Q: ?Sized;
}

impl SlotKeyFromCodec for SlotKey {
    fn new<K, Q, KC>(prefix: &Prefix, key: &Q, codec: &KC) -> Self
    where
        KC: EncodeLike<Q, K> + StateItemDecoder<K> + 'static,
        K: fmt::Display,
        Q: ?Sized,
    {
        let mut builder = SlotKeyBuilder::with_prefix(*prefix);
        codec.encode_like(key, &mut builder);
        builder.to_slot_key()
    }
}


/// A trait for creating a new value from a codec.
pub trait SlotValueFromCodec {
    /// Create a new storage value by serializing the input with the given codec.
    fn new<V, Vq, VC>(value: &Vq, codec: &VC) -> Self
    where
        Vq: ?Sized,
        VC: EncodeLike<Vq, V>;
}

impl SlotValueFromCodec for SlotValue {
    fn new<V, Vq, VC>(value: &Vq, codec: &VC) -> Self
    where
        Vq: ?Sized,
        VC: EncodeLike<Vq, V>,
    
        {
            let encoded_value = codec.encode_to_vec_like(value);
            encoded_value.into()
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

impl NodeLeafAndMaybeValue {
    /// Get the size of the value.
    pub fn size(&self) -> u32 {
        self.leaf.size
    }

    /// Create a new NodeLeafAndMaybeValue withReadType::Read.
    pub fn new_read<H: Digest<OutputSize = typenum::U32>>(value: SlotValue) -> Self {
        Self {
            leaf: NodeLeaf::make_leaf::<H>(&value),
            value: ReadType::Read(value),
        }
    }
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
    pub size: u32,
    /// The hash of the value.
    pub val_hash: [u8; 32],
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

/// A write to the accessory state.
#[derive(Debug, Clone)]
pub struct AccessoryWrite {
    /// The value to write.
    pub value: Option<SlotValue>,
}

impl AccessoryWrite {
    /// Create a new accessory write.
    pub fn new(value: Option<SlotValue>) -> Self {
        Self { value }
    }
}

#[cfg(feature = "native")]
/// An object-safe interface for retrieving values. The implementer may be storage or a cache of some kind.
pub trait StateGetter: core::fmt::Debug + Send + Sync {
    /// Get the size of the value.
    fn get_leaf(
        &self,
        namespace: ProvableNamespace,
        key: &SlotKey,
    ) -> MaybePresentValue<NodeLeafAndMaybeValue>;

    /// Get the value.
    fn get(&self, namespace: Namespace, key: &SlotKey) -> MaybePresentValue<SlotValue>;

    /// Any writes from changesets *after* (not including) the given rollup height will be ignored, as if they are absent from this reader.
    /// This is a permanent change to the getter that cannot be undone except by creating a new `StateGetter` from the original source.
    fn ignore_changes_after_height(&mut self, rollup_height: RollupHeight);

    /// Get the latest rollup height available in the getter.
    fn latest_rollup_height(&self) -> Option<RollupHeight>;

    /// Clones the state getter, returning a new type-erased object.
    fn box_clone(&self) -> Box<dyn StateGetter>;
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
    fn get_accessory(&self, _key: &SlotKey) -> Option<SlotValue>;

    /// Calculates new state root but does not commit any changes to the database.
    fn compute_state_update(
        &self,
        state_accesses: StateAccesses,
        witness: &Self::Witness,
        prev_state_root: Self::Root,
        pinned_cache: Option<PinnedCache>,
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
    ) -> anyhow::Result<Option<SlotValue>>;

    /// Get the node leaf. This method does not need to load the full value into memory.
    fn get_leaf_historical<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
        witness: &Self::Witness,
    ) -> anyhow::Result<Option<NodeLeafAndMaybeValue>>;

    /// Get the accessory value at the given version or None if the key is absent.
    fn get_accessory_historical(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
    ) -> anyhow::Result<Option<SlotValue>>;

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

    /// Get a root hash at the latest version
    fn get_latest_root_hash_unbound(&self) -> anyhow::Result<Self::Root> {
        self.get_root_hash_unbound(self.latest_version_unbound())
    }
    /// Get the latest committed value for the given key, regardless of the version number associated with this storage.
    fn get_unbound<N: CompileTimeNamespace>(&self, key: SlotKey) -> Option<SlotValue>;

    /// Iterate over all current k/v pairs with the given prefix. This method is optional and returns None if not supported by the underlying storage.
    fn maybe_iter_user_values_with_prefix(
        &self,
        prefix: SlotKey,
    ) -> anyhow::Result<Option<impl Iterator<Item = (SlotKey, SlotValue)>>>;

    /// Takes the pinned cache if one is present in this storage. See [`PinnedCache`] for more details.
    ///
    /// In the full node only, the pinned cache is passed from block to block through the storage manager.
    /// On saving the previous block, the storage manager takes its pinned block; then when it creates the storage for the *first* child block,
    /// that storage is passed along with it.
    /// If a block has multiple children (i.e. the chain has a fork at some height), the pinned cache is only passed to the first child block to be created; other children have to rebuild it from db.
    ///
    /// Note that the sequencer passes the pinned cache directly between executors without this hack, so this method is only used in the full node.
    fn try_load_saved_pinned_cache(&mut self) -> Option<PinnedCache>;
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
