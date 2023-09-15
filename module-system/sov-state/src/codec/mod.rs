//! Serialization and deserialization -related logic.

mod bcs_codec;
mod borsh_codec;
mod json_codec;
mod split_codec;

pub use bcs_codec::BcsCodec;
use borsh::BorshSerialize;
pub use borsh_codec::BorshCodec;
pub use json_codec::JsonCodec;

/// A trait for types that can serialize and deserialize values for storage
/// access.
pub trait StateValueCodec<V> {
    /// Error type that can arise during deserialization.
    type Error: std::fmt::Debug;

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
/// A good example of this is [`BorshCodec`], which knows how to serialize a
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

// In borsh, a slice is encoded the same way as a vector except in edge case where
// T is zero-sized, in which case Vec<T> is not borsh encodable.
impl<T> EncodeKeyLike<[T], Vec<T>> for BorshCodec
where
    T: BorshSerialize,
{
    fn encode_key_like(&self, borrowed: &[T]) -> Vec<u8> {
        borrowed.try_to_vec().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use proptest::collection::vec;
    use proptest::prelude::any;
    use proptest::strategy::Strategy;

    use super::*;

    fn arb_vec_i32() -> impl Strategy<Value = Vec<i32>> {
        vec(any::<i32>(), 0..2048)
    }

    proptest::proptest! {
        #[test]
        fn test_borsh_slice_encode_alike(vec in arb_vec_i32()) {
            let codec = BorshCodec;
            assert_eq!(
                <BorshCodec as EncodeKeyLike<[i32], Vec<i32>>>::encode_key_like(&codec, &vec[..]),
                codec.encode_value(&vec)
            );
        }
    }
}
