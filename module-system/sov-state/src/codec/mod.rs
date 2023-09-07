//! Serialization and deserialization -related logic.

mod bcs_codec;
mod borsh_codec;
mod json_codec;

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

pub trait EncodeLike<Ref: ?Sized, Target> {
    fn encode_like(&self, borrowed: &Ref) -> Vec<u8>;
}

// All items can be encoded like themselves
impl<C, T> EncodeLike<T, T> for C
where
    C: StateValueCodec<T>,
{
    fn encode_like(&self, borrowed: &T) -> Vec<u8> {
        self.encode_value(borrowed)
    }
}

// In borsh, a slice is encoded the same way as a vector except in edge case where
// T is zero-sized, in which case Vec<T> is not borsh encodable.
impl<T> EncodeLike<[T], Vec<T>> for BorshCodec
where
    T: BorshSerialize,
{
    fn encode_like(&self, borrowed: &[T]) -> Vec<u8> {
        borrowed.try_to_vec().unwrap()
    }
}

#[test]
fn test_borsh_slice_encode_alike() {
    let codec = BorshCodec;
    let slice = [1, 2, 3];
    let vec = vec![1, 2, 3];
    assert_eq!(
        <BorshCodec as EncodeLike<[i32], Vec<i32>>>::encode_like(&codec, &slice),
        codec.encode_value(&vec)
    );
}
