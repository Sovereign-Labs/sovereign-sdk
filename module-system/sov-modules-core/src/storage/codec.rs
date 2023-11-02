//! Encoding codec definitions

use alloc::format;
use alloc::vec::Vec;
use core::fmt;

/// A trait for types that can serialize and deserialize values for storage
/// access.
pub trait StateValueCodec<V> {
    /// Error type that can arise during deserialization.
    type Error: fmt::Debug;

    /// Serializes a value into a bytes vector.
    ///
    /// This method **must** not panic as all instances of the value type are
    /// supposed to be serializable.
    fn encode_value(&self, value: &V) -> Vec<u8>;

    /// Tries to deserialize a value from a bytes slice, and returns a
    /// [`Result`] with either the deserialized value or an error.
    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::Error>;

    /// Deserializes a value from a bytes slice.
    ///
    /// # Panics
    /// Panics if the call to [`StateValueCodec::try_decode_value`] fails. Use
    /// [`StateValueCodec::try_decode_value`] if you need to gracefully handle
    /// errors.
    fn decode_value_unwrap(&self, bytes: &[u8]) -> V {
        self.try_decode_value(bytes)
            .map_err(|err| {
                format!(
                    "Failed to decode value 0x{}, error: {:?}",
                    hex::encode(bytes),
                    err
                )
            })
            .unwrap()
    }
}

/// A trait for types that can serialize keys for storage
/// access.
///
/// Note that, unlike [`StateValueCodec`], this trait does not provide
/// deserialization logic as it's not needed nor supported.
pub trait StateKeyCodec<K> {
    /// Serializes a key into a bytes vector.
    ///
    /// # Determinism
    ///
    /// All implementations of this trait method **MUST** provide deterministic
    /// serialization behavior:
    ///
    /// 1. Equal (as defined by [`Eq`]) values **MUST** be serialized to the same
    ///    byte sequence.
    /// 2. The serialization result **MUST NOT** depend on the compilation target
    ///    and other runtime environment parameters. If that were the case, zkVM
    ///    code and native code wouldn't produce the same keys.
    fn encode_key(&self, key: &K) -> Vec<u8>;
}

/// A trait for types that can serialize keys and values, as well
/// as deserializing values for storage access.
///
/// # Type bounds
/// There are no type bounds on [`StateCodec::KeyCodec`] and
/// [`StateCodec::ValueCodec`], so they can be any type at well. That said,
/// you'll find many APIs require these two to implement [`StateKeyCodec`] and
/// [`StateValueCodec`] respectively.
pub trait StateCodec {
    /// The codec used to serialize keys. See [`StateKeyCodec`].
    type KeyCodec;
    /// The codec used to serialize and deserialize values. See
    /// [`StateValueCodec`].
    type ValueCodec;

    /// Returns a reference to the type's key codec.
    fn key_codec(&self) -> &Self::KeyCodec;
    /// Returns a reference to the type's value codec.
    fn value_codec(&self) -> &Self::ValueCodec;
}

/// A trait for codecs which know how to serialize a type `Ref` as if it were
/// some other type `Target`.
///
/// A good example of this is BorshCodec, which knows how to serialize a
/// `[T;N]` as if it were a `Vec<T>` even though the two types have different
/// encodings by default.
pub trait EncodeKeyLike<Ref: ?Sized, Target> {
    /// Encodes a reference to `Ref` as if it were a reference to `Target`.
    fn encode_key_like(&self, borrowed: &Ref) -> Vec<u8>;
}

// All items can be encoded like themselves by all codecs
impl<C, T> EncodeKeyLike<T, T> for C
where
    C: StateKeyCodec<T>,
{
    fn encode_key_like(&self, borrowed: &T) -> Vec<u8> {
        self.encode_key(borrowed)
    }
}
