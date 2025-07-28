//! Serialization and deserialization -related logic.

mod bcs_codec;
mod borsh_codec;

use core::fmt;

pub use bcs_codec::BcsCodec;
pub use borsh_codec::BorshCodec;

/// A trait for types that can serialize and deserialize values for storage
/// access.
pub trait StateItemCodec<V>: StateItemEncoder<V> + StateItemDecoder<V> {}

/// A trait for types that can serialize values into storage.
pub trait StateItemEncoder<V: ?Sized> {
    /// Serializes a value into a bytes vector.
    ///
    /// This method **must** not panic as all instances of the value type are
    /// supposed to be serializable.
    fn encode(&self, value: &V) -> Vec<u8>;
}

/// A trait for types that can deserialize values from storage.
pub trait StateItemDecoder<V>: Clone + Send + Sync + 'static {
    /// Error type that can arise during deserialization.
    type Error: fmt::Debug;

    /// Tries to deserialize a value from a bytes slice, and returns a
    /// [`Result`] with either the deserialized value or an error.
    fn try_decode(&self, bytes: &[u8]) -> Result<V, Self::Error>;

    /// Deserializes a value from a bytes slice.
    ///
    /// # Panics
    /// Panics if the call to [`StateItemDecoder::try_decode`] fails. Use
    /// [`StateItemDecoder::try_decode`] if you need to gracefully handle
    /// errors.
    fn decode_unwrap(&self, bytes: &[u8]) -> V {
        self.try_decode(bytes)
            .map_err(|err| {
                format!(
                    "Failed to decode {:?} value 0x{}, error: {:?}",
                    core::any::type_name::<V>(),
                    hex::encode(bytes),
                    err
                )
            })
            .unwrap()
    }
}

impl<C, V> StateItemCodec<V> for C where C: StateItemEncoder<V> + StateItemDecoder<V> {}

/// A trait for types that can serialize keys and values, as well
/// as deserializing values for storage access.
///
/// # Type bounds
/// There are no type bounds on [`StateCodec::KeyCodec`] and
/// [`StateCodec::ValueCodec`], so they can be any type at well. That said,
/// you'll find many APIs require these two to implement [`StateItemCodec`] and
/// [`StateItemCodec`] respectively.
pub trait StateCodec: Default + Clone + Send + Sync + 'static {
    /// The codec used to serialize keys. See [`StateItemCodec`].
    type KeyCodec;
    /// The codec used to serialize and deserialize values. See
    /// [`StateItemCodec`].
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
/// `[T;N]` as if it were a [`Vec<T>`] even though the two types have different
/// encodings by default.
pub trait EncodeLike<Ref: ?Sized, Target>: StateItemEncoder<Target> {
    /// Encodes a reference to `Ref` as if it were a reference to `Target`.
    fn encode_like(&self, borrowed: &Ref) -> Vec<u8>;
}

/// All items can be encoded like themselves by all codecs.
impl<C, T> EncodeLike<T, T> for C
where
    C: StateItemCodec<T>,
{
    fn encode_like(&self, borrowed: &T) -> Vec<u8> {
        self.encode(borrowed)
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

    #[test_strategy::proptest]
    fn test_borsh_slice_encode_alike(#[strategy(arb_vec_i32())] vec: Vec<i32>) {
        let codec = BorshCodec;
        assert_eq!(
            <BorshCodec as EncodeLike<[i32], Vec<i32>>>::encode_like(&codec, &vec[..]),
            codec.encode(&vec)
        );
    }
}
