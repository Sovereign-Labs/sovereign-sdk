//! Module storage definitions.

use core::fmt;
use std::fmt::Display;
use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use derivative::Derivative;
use jmt::KeyHash;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
#[cfg(feature = "native")]
use sov_rollup_interface::common::{RollupHeight, SlotNumber};
use sov_rollup_interface::reexports::digest::{typenum, Digest};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

use crate::bytes::Prefix;
use crate::codec::EncodeLike;
use crate::namespaces::{ProvableCompileTimeNamespace, ProvableNamespace};
#[cfg(feature = "native")]
use crate::sequencer_state::MaybePresentValue;
#[cfg(feature = "native")]
use crate::{CompileTimeNamespace, Namespace};
use crate::{
    MerkleProofSpec, SparseMerkleProof, StateAccesses, StateItemDecoder, StorageRoot, Witness,
};

/// The key type suitable for use in [`Storage::get`] and other getter methods of
/// [`Storage`]. Cheaply-clonable.
#[derive(
    Derivative, Serialize, serde::Deserialize, BorshDeserialize, BorshSerialize, UniversalWallet,
)]
#[derivative(Clone, PartialEq, Eq, Debug, Hash, Ord)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct SlotKey {
    // // TODO: Handle extra long keys
    // pub(crate) prefix: Prefix,
    key: KeyContents,
}

/// A logical key consists of [module_tag, item_tag, serialized_key]
/// We have two different *physical* representations of the key in memory.
///    If serialized_key length is less than 79 bytes, we store the key inline.
///    With the representation length_bytes || module_tag || item_tag || serialized_key.
///       AsRef<[u8]> returns module_tag || item_tag || serialized_key
///
///    If the serialized_key length is greater than 79 bytes, we store the key as Vec<u8> containing [module_tag, item_tag, serialized_key].
///
/// When we store a slot key on disk, we store the physical representation
#[derive(
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
    UniversalWallet,
    Debug,
    PartialEq,
    Eq,
    Hash,
    Ord,
    PartialOrd,
    Clone,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
enum KeyContents {
    /// A key layed out as follows:
    /// [len_byte, module_tag, item_tag, key_bytes]
    Inline(private::InlineKey),
    /// A key layed out as follows:
    Reference(Arc<Vec<u8>>),
}

mod private {
    use borsh::{BorshDeserialize, BorshSerialize};
    use serde::{Deserialize, Serialize};
    use serde_with::{serde_as, Bytes};
    use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

    use crate::Prefix;

    /// The total number of bytes in an inline key.
    ///
    /// Choose this value to be long enough for a balance key
    /// This is long enough to fit a token balance key (which consists of a 32 byte address and a 32 byte amount, each with an associated tag byte) pluse
    /// 2 bytes of prefix and one byte of metadata. That gives 69 bytes; round up to 70 in case we forgot anything.
    const INLINE_KEY_BYTES: usize = 70;
    /// The number of bytes in the metadata of an inline key
    const INLINE_KEY_METADATA_BYTES: usize = 3;
    /// The maximum length of key material (excluding prefix) that can be stored in an inline key
    const MAX_INLINE_KEY_LEN: usize = INLINE_KEY_BYTES - INLINE_KEY_METADATA_BYTES;

    const MODULE_TAG_OFFSET: usize = 1;
    const ITEM_TAG_OFFSET: usize = 2;

    #[serde_as]
    #[derive(
        Serialize,
        Deserialize,
        BorshSerialize,
        BorshDeserialize,
        Clone,
        PartialEq,
        Eq,
        Debug,
        Hash,
        Ord,
        PartialOrd,
        UniversalWallet,
    )]
    pub(super) struct InlineKey(
        #[serde_as(as = "Bytes")]
        /// Layout: [len_byte, module_tag, item_tag, key_bytes]
        [u8; INLINE_KEY_BYTES],
    );

    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct ErrOutOfSpace;

    fn offset(idx: usize) -> usize {
        3 + idx
    }

    impl InlineKey {
        /// By definition, the length of the key is the first byte
        pub(super) fn len_excluding_prefix(&self) -> usize {
            self.0[0] as usize
        }

        #[cfg(test)]
        pub(super) fn module_tag(&self) -> u8 {
            self.0[MODULE_TAG_OFFSET]
        }

        #[cfg(test)]
        pub(super) fn item_tag(&self) -> u8 {
            self.0[ITEM_TAG_OFFSET]
        }

        pub(super) fn without_prefix(&self) -> &[u8] {
            &self.0[offset(0)..offset(self.len_excluding_prefix())]
        }

        pub(super) fn with_prefix(&self) -> &[u8] {
            &self.0[1..offset(self.len_excluding_prefix())]
        }

        pub fn try_extend(&mut self, other: &[u8]) -> Result<(), ErrOutOfSpace> {
            let original_len = self.len_excluding_prefix();
            let new_len = original_len.checked_add(other.len()).ok_or(ErrOutOfSpace)?;
            if new_len > MAX_INLINE_KEY_LEN {
                return Err(ErrOutOfSpace);
            }
            self.0[0] = new_len.try_into().expect("Overflow checking length - this should be unreachable since we've already checked the length");
            self.0[offset(original_len)..offset(new_len)].copy_from_slice(other);
            Ok(())
        }

        pub fn try_from_prefix_and_key(prefix: Prefix, key: &[u8]) -> Result<Self, ErrOutOfSpace> {
            if key.len() > MAX_INLINE_KEY_LEN {
                return Err(ErrOutOfSpace);
            }
            let mut output = Self([0u8; INLINE_KEY_BYTES]);
            output.0[MODULE_TAG_OFFSET] = prefix.module;
            output.0[ITEM_TAG_OFFSET] = prefix.item;
            output.try_extend(key)?;
            Ok(output)
        }
    }

    #[cfg(feature = "arbitrary")]
    impl arbitrary::Arbitrary<'_> for InlineKey {
        fn arbitrary(u: &mut arbitrary::Unstructured) -> arbitrary::Result<Self> {
            let len = u.int_in_range(0..=MAX_INLINE_KEY_LEN)?;
            let key = u.bytes(len)?;
            Ok(Self::try_from_prefix_and_key(Prefix::new(0, 0), key).unwrap())
        }
    }

    #[test]
    fn test_inline_key_extend() {
        let mut key = InlineKey([0u8; INLINE_KEY_BYTES]);
        key.try_extend(&[1, 2, 3]).unwrap();
        assert_eq!(key.len_excluding_prefix(), 3);
        assert_eq!(key.without_prefix(), &[1, 2, 3]);
        assert_eq!(key.module_tag(), 0);
        assert_eq!(key.item_tag(), 0);
        assert_eq!(key.0[..6], [3, 0, 0, 1, 2, 3]);

        key.try_extend(&[4, 5, 6]).unwrap();
        assert_eq!(key.len_excluding_prefix(), 6);
        assert_eq!(key.without_prefix(), &[1, 2, 3, 4, 5, 6]);
        assert_eq!(key.module_tag(), 0);
        assert_eq!(key.item_tag(), 0);
        assert_eq!(key.0[..9], [6, 0, 0, 1, 2, 3, 4, 5, 6]);

        // We've written 6 bytes so far, so we have `REMAINING_BYTES` of space left inline.
        const REMAINING_BYTES: usize = MAX_INLINE_KEY_LEN - 6;

        assert!(key.try_extend(&[1; (REMAINING_BYTES + 1)]).is_err());
        assert_eq!(key.len_excluding_prefix(), 6);
        assert_eq!(key.0[..9], [6, 0, 0, 1, 2, 3, 4, 5, 6]);

        assert!(key.try_extend(&[1; REMAINING_BYTES]).is_ok());
        assert_eq!(key.len_excluding_prefix(), MAX_INLINE_KEY_LEN);
        assert_eq!(
            key.0[..9],
            [MAX_INLINE_KEY_LEN as u8, 0, 0, 1, 2, 3, 4, 5, 6]
        );
        assert_eq!(&key.without_prefix()[6..], &[1; REMAINING_BYTES]);
    }
}

// Manually implement PartialOrd to satisfy clippy
impl PartialOrd for SlotKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl SlotKey {
    /// Returns a new [`Arc`] reference to the bytes of this key excluding the prefix.
    pub fn without_prefix(&self) -> &[u8] {
        match &self.key {
            KeyContents::Inline(key) => key.without_prefix(),
            KeyContents::Reference(key) => &key.as_ref()[2..],
        }
    }

    /// Returns a new [`Arc`] reference to the bytes of this key including the prefix
    pub fn with_prefix(&self) -> &[u8] {
        match &self.key {
            KeyContents::Inline(key) => key.with_prefix(),
            KeyContents::Reference(key) => &key.as_ref()[..],
        }
    }

    /// Returns the length of the key including the prefix.
    // Is empty would always return false because of the prefix, so we don't provide it.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        match &self.key {
            KeyContents::Inline(key) => key.len_excluding_prefix() + 2,
            KeyContents::Reference(key) => key.len(),
        }
    }

    /// Craetes a test key with the given byte.
    pub fn test_key(byte: u8) -> Self {
        Self {
            key: KeyContents::Inline(
                private::InlineKey::try_from_prefix_and_key(Prefix::new(byte, byte), &[byte])
                    .unwrap(),
            ),
        }
    }
}

impl fmt::Display for SlotKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key: &[u8] = self.as_ref();
        write!(f, "{}", hex::encode(key))
    }
}

impl AsRef<[u8]> for SlotKey {
    fn as_ref(&self) -> &[u8] {
        match &self.key {
            KeyContents::Inline(key) => key.with_prefix(),
            KeyContents::Reference(key) => key.as_ref(),
        }
    }
}

enum SlotKeyBuilder {
    Inline(private::InlineKey),
    Reference(Vec<u8>),
}

impl From<SlotKeyBuilder> for KeyContents {
    fn from(builder: SlotKeyBuilder) -> Self {
        match builder {
            SlotKeyBuilder::Inline(key) => KeyContents::Inline(key),
            SlotKeyBuilder::Reference(key) => KeyContents::Reference(Arc::new(key)),
        }
    }
}

impl SlotKeyBuilder {
    pub fn with_prefix(prefix: Prefix) -> Self {
        Self::Inline(private::InlineKey::try_from_prefix_and_key(prefix, &[]).unwrap())
    }
}

impl std::io::Write for SlotKeyBuilder {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            SlotKeyBuilder::Inline(key) => {
                if key.try_extend(buf).is_err() {
                    let mut new_key =
                        Vec::with_capacity(key.len_excluding_prefix() + buf.len() + 2);
                    new_key.extend(key.with_prefix());
                    new_key.extend(buf);
                    *self = SlotKeyBuilder::Reference(new_key);
                }
            }
            SlotKeyBuilder::Reference(key) => {
                key.write(buf)?;
            }
        };

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
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
        let mut builder = SlotKeyBuilder::with_prefix(*prefix);
        codec.encode_like(key, &mut builder);
        Self {
            key: builder.into(),
        }
    }

    /// Used only in tests.
    /// Builds a storage key from a byte slice
    pub fn from_slice(key: &[u8]) -> Self {
        use std::io::Write;
        let mut builder = SlotKeyBuilder::with_prefix(Prefix::new(0, 0));
        builder.write_all(key).unwrap();
        Self {
            key: builder.into(),
        }
    }

    /// Creates a new [`SlotKey`] from a prefix.
    pub fn singleton(prefix: &Prefix) -> Self {
        let builder = SlotKeyBuilder::with_prefix(*prefix).into();
        Self { key: builder }
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
        let encoded_value = codec.encode_to_vec_like(value);
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
