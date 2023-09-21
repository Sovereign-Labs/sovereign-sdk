use std::fmt::Display;
use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use hex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_first_read_last_write_cache::{CacheKey, CacheValue};

use crate::codec::{EncodeKeyLike, StateValueCodec};
use crate::internal_cache::OrderedReadsAndWrites;
use crate::utils::AlignedVec;
use crate::witness::Witness;
use crate::Prefix;

/// The key type suitable for use in [`Storage::get`] and other getter methods of
/// [`Storage`]. Cheaply-clonable.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
pub struct StorageKey {
    key: Arc<Vec<u8>>,
}

impl From<CacheKey> for StorageKey {
    fn from(cache_key: CacheKey) -> Self {
        Self { key: cache_key.key }
    }
}

impl StorageKey {
    /// Returns a new [`Arc`] reference to the bytes of this key.
    pub fn key(&self) -> Arc<Vec<u8>> {
        self.key.clone()
    }

    /// Converts this key into a [`CacheKey`] via cloning.
    pub fn to_cache_key(&self) -> CacheKey {
        CacheKey {
            key: self.key.clone(),
        }
    }

    /// Converts this key into a [`CacheKey`].
    pub fn into_cache_key(self) -> CacheKey {
        CacheKey { key: self.key }
    }
}

impl AsRef<Vec<u8>> for StorageKey {
    fn as_ref(&self) -> &Vec<u8> {
        &self.key
    }
}

impl Display for StorageKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x?}", hex::encode(self.key().as_ref()))
    }
}

impl StorageKey {
    /// Creates a new [`StorageKey`] that combines a prefix and a key.
    pub fn new<K, Q, KC>(prefix: &Prefix, key: &Q, codec: &KC) -> Self
    where
        KC: EncodeKeyLike<Q, K>,
        Q: ?Sized,
    {
        let encoded_key = codec.encode_key_like(key);
        let encoded_key = AlignedVec::new(encoded_key);

        let full_key = Vec::<u8>::with_capacity(prefix.len() + encoded_key.len());
        let mut full_key = AlignedVec::new(full_key);
        full_key.extend(prefix.as_aligned_vec());
        full_key.extend(&encoded_key);

        Self {
            key: Arc::new(full_key.into_inner()),
        }
    }

    /// Creates a new [`StorageKey`] that combines a prefix and a key.
    pub fn singleton(prefix: &Prefix) -> Self {
        Self {
            key: Arc::new(prefix.as_aligned_vec().clone().into_inner()),
        }
    }
}

/// A serialized value suitable for storing. Internally uses an [`Arc<Vec<u8>>`]
/// for cheap cloning.
#[derive(
    Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default,
)]
pub struct StorageValue {
    value: Arc<Vec<u8>>,
}

impl From<CacheValue> for StorageValue {
    fn from(cache_value: CacheValue) -> Self {
        Self {
            value: cache_value.value,
        }
    }
}

impl From<Vec<u8>> for StorageValue {
    fn from(value: Vec<u8>) -> Self {
        Self {
            value: Arc::new(value),
        }
    }
}

impl StorageValue {
    /// Create a new storage value by serializing the input with the given codec.
    pub fn new<V, VC>(value: &V, codec: &VC) -> Self
    where
        VC: StateValueCodec<V>,
    {
        let encoded_value = codec.encode_value(value);
        Self {
            value: Arc::new(encoded_value),
        }
    }

    /// Get the bytes of this value.
    pub fn value(&self) -> &[u8] {
        &self.value
    }

    /// Convert this value into a [`CacheValue`].
    pub fn into_cache_value(self) -> CacheValue {
        CacheValue { value: self.value }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
/// A proof that a particular storage key has a particular value, or is absent.
pub struct StorageProof<P> {
    /// The key which is proven
    pub key: StorageKey,
    /// The value, if any, which is proven
    pub value: Option<StorageValue>,
    /// The cryptographic proof
    pub proof: P,
}

/// An interface for storing and retrieving values in the storage.
pub trait Storage: Clone {
    /// The witness type for this storage instance.
    type Witness: Witness;

    /// The runtime config for this storage instance.
    type RuntimeConfig;

    /// A cryptographic proof that a particular key has a particular value, or is absent.
    type Proof: Serialize
        + DeserializeOwned
        + core::fmt::Debug
        + Clone
        + BorshSerialize
        + BorshDeserialize;

    /// A cryptographic commitment to the contents of this storage
    type Root: Serialize
        + DeserializeOwned
        + core::fmt::Debug
        + Clone
        + BorshSerialize
        + BorshDeserialize
        + Eq
        + Into<[u8; 32]>; // Require a one-way conversion from the state root to a 32-byte array. This can always be
                          // implemented by hashing the state root even if the root itself is not 32 bytes.

    /// State update that will be committed to the database.
    type StateUpdate;

    /// Creates a new instance of this [`Storage`] type, with some configuration
    /// options.
    fn with_config(config: Self::RuntimeConfig) -> Result<Self, anyhow::Error>;

    /// Returns the value corresponding to the key or None if key is absent.
    fn get(&self, key: &StorageKey, witness: &Self::Witness) -> Option<StorageValue>;

    /// Returns the value corresponding to the key or None if key is absent.
    ///
    /// # About accessory state
    /// This method is blanket-implemented to return [`None`]. **Only native
    /// execution environments** (i.e. outside of the zmVM) **SHOULD** override
    /// this method to return a value. This is because accessory state **MUST
    /// NOT** be readable from within the zmVM.
    fn get_accessory(&self, _key: &StorageKey) -> Option<StorageValue> {
        None
    }

    /// Calculates new state root but does not commit any changes to the database.
    fn compute_state_update(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
    ) -> Result<(Self::Root, Self::StateUpdate), anyhow::Error>;

    /// Commits state changes to the database.
    fn commit(&self, node_batch: &Self::StateUpdate, accessory_update: &OrderedReadsAndWrites);

    /// Validate all of the storage accesses in a particular cache log,
    /// returning the new state root after applying all writes.
    /// This function is equivalent to calling:
    /// `self.compute_state_update & self.commit`
    fn validate_and_commit(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
    ) -> Result<Self::Root, anyhow::Error> {
        Self::validate_and_commit_with_accessory_update(
            self,
            state_accesses,
            witness,
            &Default::default(),
        )
    }

    /// A version of [`Storage::validate_and_commit`] that allows for
    /// "accessory" non-JMT updates. See `sov_db::NativeDB` for more information
    /// about accessory state.
    fn validate_and_commit_with_accessory_update(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
        accessory_update: &OrderedReadsAndWrites,
    ) -> Result<Self::Root, anyhow::Error> {
        let (root_hash, node_batch) = self.compute_state_update(state_accesses, witness)?;
        self.commit(&node_batch, accessory_update);

        Ok(root_hash)
    }

    /// Opens a storage access proof and validates it against a state root.
    /// It returns a result with the opened leaf (key, value) pair in case of success.
    fn open_proof(
        &self,
        state_root: Self::Root,
        proof: StorageProof<Self::Proof>,
    ) -> Result<(StorageKey, Option<StorageValue>), anyhow::Error>;

    /// Indicates if storage is empty or not.
    /// Useful during initialization.
    fn is_empty(&self) -> bool;
}

// Used only in tests.
impl From<&str> for StorageKey {
    fn from(key: &str) -> Self {
        Self {
            key: Arc::new(key.as_bytes().to_vec()),
        }
    }
}

// Used only in tests.
impl From<&str> for StorageValue {
    fn from(value: &str) -> Self {
        Self {
            value: Arc::new(value.as_bytes().to_vec()),
        }
    }
}

/// A [`Storage`] that is suitable for use in native execution environments
/// (outside of the zkVM).
pub trait NativeStorage: Storage {
    /// Returns the value corresponding to the key or None if key is absent and a proof to
    /// get the value.
    fn get_with_proof(&self, key: StorageKey, witness: &Self::Witness)
        -> StorageProof<Self::Proof>;
}
