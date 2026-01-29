use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use derivative::Derivative;
use digest::typenum;
use digest::Digest;
#[cfg(feature = "native")]
use rockbound::versioned_db::HasPrefix;
#[cfg(feature = "native")]
use rockbound::versioned_db::VersionedSchemaKeyMarker;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

use core::{fmt, str};

/// A prefix prepended to each key before insertion and retrieval from the storage.
///
/// When interacting with state containers, you will usually use the same working set instance to
/// access them, as required by the module API. This also means that you might get key collisions,
/// so it becomes necessary to prepend a prefix to each key.
#[derive(
    Debug,
    PartialEq,
    Eq,
    Clone,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    UniversalWallet,
    Copy,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
pub struct Prefix {
    pub(crate) module: u8,
    pub(crate) item: u8,
}

impl Prefix {
    /// Returns a new [`Prefix`] with the item offset by the given amount.
    pub fn offset_by(self, offset: u8) -> Self {
        Prefix {
            module: self.module,
            item: self.item.checked_add(offset).expect("Overflow checking offset - this should be unreachable since we've already checked the offset"),
        }
    }
    /// Returns the module short id of the [`Prefix`].
    pub fn module(&self) -> u8 {
        self.module
    }
    /// Returns the item id of the [`Prefix`].
    pub fn item(&self) -> u8 {
        self.item
    }

    /// Returns a new [`Prefix`] with the given module and item discriminants.
    pub fn new(module: u8, item: u8) -> Self {
        Self { module, item }
    }
}

impl fmt::Display for Prefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let buf = [self.module, self.item];
        write!(f, "0x{}", hex::encode(buf))
    }
}

mod private {
    use super::Prefix;
    use borsh::{BorshDeserialize, BorshSerialize};
    use serde::{Deserialize, Serialize};
    use serde_with::{serde_as, Bytes};
    use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

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

        pub(crate) fn module_tag(&self) -> u8 {
            self.0[MODULE_TAG_OFFSET]
        }

        pub(crate) fn item_tag(&self) -> u8 {
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
            let len = u.int_in_range(2..=MAX_INLINE_KEY_LEN)?;
            let key = u.bytes(len)?;
            let mut output = Self([0u8; INLINE_KEY_BYTES]);
            output.0[..key.len()].copy_from_slice(key);
            Ok(output)
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

    #[test]
    fn test_to_vec_and_back() {
        use super::SlotKey;
        use super::SlotKeyBuilder;
        use std::io::Write;
        let mut builder = SlotKeyBuilder::with_prefix(Prefix::new(9, 8));
        builder.write_all(&[1, 2, 3]).unwrap();
        let key: SlotKey = SlotKey {
            key: builder.into(),
        };
        let vec = key.as_ref().to_vec();
        let key2 = SlotKey::from_slice_including_prefix(&vec);
        assert_eq!(key, key2);

        let mut builder = SlotKeyBuilder::with_prefix(Prefix::new(9, 8));
        builder.write_all(&[1; 100]).unwrap();
        let key: SlotKey = SlotKey {
            key: builder.into(),
        };
        let vec = key.as_ref().to_vec();
        let key2 = SlotKey::from_slice_including_prefix(&vec);
        assert_eq!(key, key2);
    }
}

/// The key type suitable for use in [`Storage::get`] and other getter methods of
/// [`Storage`]. Cheaply-clonable and cache friendly.
///
/// Semantically, a slot key is just a byte slice where the first two bytes are the prefix and the rest are the key.
/// Physically, we store the key as an enum of an inline key and an Arc<Vec<u8>>. Keys up to 70 bytes are stored on the stack
/// (no allocation, no cache misses to deref), while larger keys are stored on the heap.
#[derive(Derivative, UniversalWallet)]
#[derivative(Clone, Debug)]
pub struct SlotKey {
    key: KeyContents,
}

#[cfg(feature = "native")]
impl VersionedSchemaKeyMarker for SlotKey {}
#[cfg(feature = "native")]
impl HasPrefix for SlotKey {
    fn has_prefix(&self, prefix: &Self) -> bool {
        self.as_ref().starts_with(prefix.as_ref())
    }
}

// Because slot key has an optimized memory layout to avoid allocation when possible, we need to manually implement pretty much every trait.
// We always want to treat the slot key as if it were a byte slice,
mod slot_key_inner {
    use super::Prefix;
    use super::SlotKey;
    use super::SlotKeyBuilder;
    use std::hash::{Hash, Hasher};

    impl PartialEq for SlotKey {
        fn eq(&self, other: &Self) -> bool {
            self.as_ref() == other.as_ref()
        }
    }

    impl Eq for SlotKey {}

    impl Hash for SlotKey {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.as_ref().hash(state);
        }
    }

    impl Ord for SlotKey {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.as_ref().cmp(other.as_ref())
        }
    }

    impl PartialOrd for SlotKey {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    #[cfg(feature = "arbitrary")]
    impl arbitrary::Arbitrary<'_> for SlotKey {
        fn arbitrary(u: &mut arbitrary::Unstructured) -> arbitrary::Result<Self> {
            use std::io::Write;
            let prefix = Prefix::arbitrary(u)?;
            let len = u.arbitrary_len::<u8>()? + 2;
            let bytes = u.bytes(len)?;
            let mut build = SlotKeyBuilder::with_prefix(prefix);
            build
                .write_all(bytes)
                .expect("Failed to write bytes to slot key builder");
            Ok(Self { key: build.into() })
        }
    }

    impl serde::Serialize for SlotKey {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            self.as_ref().serialize(serializer)
        }
    }

    impl<'de> serde::Deserialize<'de> for SlotKey {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            use std::io::Write;
            let bytes = Vec::<u8>::deserialize(deserializer)?;
            if bytes.len() < 2 {
                return Err(serde::de::Error::custom(
                    "Slot key must be at least 2 bytes",
                ));
            }
            let prefix = Prefix::new(bytes[0], bytes[1]);
            let mut builder = SlotKeyBuilder::with_prefix(prefix);
            builder
                .write_all(&bytes[2..])
                .expect("Failed to write bytes to slot key builder");
            Ok(Self {
                key: builder.into(),
            })
        }
    }

    impl borsh::BorshSerialize for SlotKey {
        fn serialize<W>(&self, writer: &mut W) -> Result<(), std::io::Error>
        where
            W: std::io::Write,
        {
            self.as_ref().serialize(writer)
        }
    }

    impl borsh::BorshDeserialize for SlotKey {
        fn deserialize_reader<R>(reader: &mut R) -> Result<Self, std::io::Error>
        where
            R: std::io::Read,
        {
            use std::io::Write;
            let bytes = Vec::<u8>::deserialize_reader(reader)?;
            if bytes.len() < 2 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Slot key must be at least 2 bytes",
                ));
            }
            let prefix = Prefix::new(bytes[0], bytes[1]);
            let mut builder = SlotKeyBuilder::with_prefix(prefix);
            builder.write_all(&bytes[2..])?;
            Ok(Self {
                key: builder.into(),
            })
        }
    }

    #[test]
    fn test_as_ref() {
        let prefix = Prefix::new(1, 2);
        let key = build_key(prefix, b"hello");
        let key_ref = key.as_ref();
        assert_eq!(key_ref, b"\x01\x02hello");
    }

    #[test]
    fn test_borsh_roundtrip() {
        let prefix = Prefix::new(1, 2);
        let key = build_key(prefix, b"hello");
        let serialized = borsh::to_vec(&key).unwrap();
        let key2: SlotKey = borsh::from_slice(&serialized).unwrap();

        assert_eq!(key.prefix(), prefix);
        assert_eq!(key.prefix(), key2.prefix());
        assert_eq!(key, key2);
    }

    #[test]
    fn test_serde_roundtrip() {
        let prefix = Prefix::new(1, 2);
        let key = build_key(prefix, b"hello");
        let json = serde_json::to_string(&key).unwrap();
        let key2: SlotKey = serde_json::from_str(&json).unwrap();
        assert_eq!(key.prefix(), prefix);
        assert_eq!(key.prefix(), key2.prefix());
        assert_eq!(key, key2);
    }

    #[cfg(test)]
    fn build_key(prefix: Prefix, key: &[u8]) -> SlotKey {
        use std::io::Write;
        let mut builder = SlotKeyBuilder::with_prefix(prefix);
        builder.write_all(key).unwrap();
        SlotKey {
            key: builder.into(),
        }
    }

    #[test]
    fn test_comparison() {
        let prefix = Prefix::new(1, 2);
        let key1 = build_key(prefix, b"hello");
        let key2 = build_key(prefix, b"goodbye");
        assert!(key1 != key2);
        assert!(key1 > key2);

        let key3 = build_key(Prefix::new(1, 2), &[0u8; 500]);
        let key4 = build_key(Prefix::new(1, 2), &[1]);

        assert!(key3 != key4);
        assert!(key3 < key4);
        assert!(key3 < key1);

        let key5 = build_key(Prefix::new(1, 2), b"hello");
        assert!(key5 == key1);

        let lowest_key = build_key(Prefix::new(0, 0), b"xylophone");
        assert!(lowest_key < key2);
        assert!(lowest_key < key3);
    }

    #[test]
    fn test_truncate_to() {
        let key = build_key(Prefix::new(1, 2), b"hello");
        let long = build_key(Prefix::new(1, 2), b"helloworld");
        assert_eq!(long.truncate_to(5).unwrap(), key);

        let key = build_key(Prefix::new(1, 2), &[2u8; 500]);
        let key2 = build_key(Prefix::new(1, 2), &[2u8; 7]);
        assert_eq!(key.truncate_to(7).unwrap(), key2);
    }
}

/// A logical key consists of [module_tag, item_tag, serialized_key]
/// We have two different *physical* representations of the key in memory.
/// - If serialized_key length is less than 79 bytes, we store the key inline.
///   With the representation length_bytes || module_tag || item_tag || serialized_key.
/// - If the serialized_key length is greater than 79 bytes, we store the key as Vec<u8> containing [module_tag, item_tag, serialized_key].
///
/// When we store a slot key on disk, we store the physical representation
#[derive(
    Serialize, Deserialize, BorshSerialize, BorshDeserialize, UniversalWallet, Debug, Clone,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
enum KeyContents {
    /// A key layed out as follows:
    /// [len_byte, module_tag, item_tag, key_bytes]
    Inline(private::InlineKey),
    /// A key layed out as follows:
    Reference(Arc<Vec<u8>>),
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

    /// Creates a new [`SlotKey`] from a vector of bytes including the prefix.
    ///
    /// # Panics
    /// Panics if the vector length is less than 2 bytes.
    pub fn from_slice_including_prefix(vec: &[u8]) -> Self {
        use std::io::Write;
        let prefix = Prefix::new(vec[0], vec[1]);
        let mut builder = SlotKeyBuilder::with_prefix(prefix);
        builder.write_all(&vec[2..]).unwrap();
        Self {
            key: builder.into(),
        }
    }

    /// Returns the prefix of the key.
    pub fn prefix(&self) -> Prefix {
        match &self.key {
            KeyContents::Inline(key) => Prefix::new(key.module_tag(), key.item_tag()),
            KeyContents::Reference(key) => Prefix::new(key[0], key[1]),
        }
    }

    /// Returns a new key containing the first `length_excluding_prefix` bytes of the original key.
    /// The truncated key will return the original prefix.
    pub fn truncate_to(&self, length_excluding_prefix: usize) -> Option<Self> {
        use std::io::Write;
        if length_excluding_prefix > self.len().checked_sub(2)? {
            return None;
        }
        let mut output = SlotKeyBuilder::with_prefix(self.prefix());
        output
            .write_all(&self.without_prefix()[..length_excluding_prefix])
            .unwrap();
        Some(SlotKey { key: output.into() })
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

/// A builder for [`SlotKey`].
pub enum SlotKeyBuilder {
    #[allow(missing_docs)]
    #[allow(private_interfaces)]
    Inline(private::InlineKey),
    #[allow(missing_docs)]
    Reference(Vec<u8>),
}

impl SlotKeyBuilder {
    /// Creates a new builder with the given prefix.
    pub fn with_prefix(prefix: Prefix) -> Self {
        Self::Inline(private::InlineKey::try_from_prefix_and_key(prefix, &[]).unwrap())
    }

    /// Converts the builder into a [`SlotKey`].
    pub fn to_slot_key(self) -> SlotKey {
        SlotKey { key: self.into() }
    }
}

impl From<SlotKeyBuilder> for KeyContents {
    fn from(builder: SlotKeyBuilder) -> Self {
        match builder {
            SlotKeyBuilder::Inline(key) => KeyContents::Inline(key),
            SlotKeyBuilder::Reference(key) => KeyContents::Reference(Arc::new(key)),
        }
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
    /// Used only in tests.
    /// Builds a storage key from a byte slice
    pub fn from_slice(key: &[u8]) -> Self {
        use std::io::Write;
        let mut builder = SlotKeyBuilder::with_prefix(Prefix::new(key[0], key[1]));
        builder.write_all(&key[2..]).unwrap();
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

/// Used only in tests.
impl From<&str> for SlotValue {
    fn from(value: &str) -> Self {
        Self {
            value: Arc::new(value.as_bytes().to_vec()),
        }
    }
}

impl AsRef<[u8]> for SlotValue {
    fn as_ref(&self) -> &[u8] {
        self.value.as_ref()
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
    pub fn combine_val_hash_and_size<H: Digest<OutputSize = typenum::U32>>(&self) -> Vec<u8> {
        let val_hash: [u8; 32] = H::digest(self.value.as_ref()).into();
        val_hash_and_size_inner(val_hash, self.size())
    }
}

/// Combines the value hash with its size.
// We return `Vec<u8>` here to be compatible with the `JMT::put_value_set_with_proof` method.
pub fn val_hash_and_size_inner(val_hash: [u8; 32], size: u32) -> Vec<u8> {
    let mut val_hash_and_size = Vec::with_capacity(40);
    let size_bytes = size.to_le_bytes();
    val_hash_and_size.extend_from_slice(&val_hash);
    val_hash_and_size.extend_from_slice(&size_bytes);
    val_hash_and_size
}
