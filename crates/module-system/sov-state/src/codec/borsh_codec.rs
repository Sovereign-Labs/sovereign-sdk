use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::common::HexString;

use super::{EncodeLike, StateCodec, StateItemDecoder, StateItemEncoder};

/// A [`StateCodec`] that uses [`borsh`] for all keys and values.
#[derive(
    Debug, Default, PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct BorshCodec;

impl<V> StateItemEncoder<V> for BorshCodec
where
    V: BorshSerialize + ?Sized,
{
    fn encode(&self, value: &V) -> Vec<u8> {
        borsh::to_vec(value).expect("Failed to serialize value")
    }
}

impl<V> StateItemDecoder<V> for BorshCodec
where
    V: BorshDeserialize,
{
    type Error = std::io::Error;

    fn try_decode(&self, bytes: &[u8]) -> Result<V, Self::Error> {
        V::try_from_slice(bytes)
    }
}

impl StateCodec for BorshCodec {
    type KeyCodec = Self;
    type ValueCodec = Self;

    fn key_codec(&self) -> &Self::KeyCodec {
        self
    }

    fn value_codec(&self) -> &Self::ValueCodec {
        self
    }
}

// In borsh, a slice is encoded the same way as a vector except in edge case where
// T is zero-sized, in which case Vec<T> is not borsh encodable.
impl<T> EncodeLike<[T], Vec<T>> for BorshCodec
where
    T: BorshSerialize,
{
    fn encode_like(&self, borrowed: &[T]) -> Vec<u8> {
        borsh::to_vec(borrowed).expect("Borsh serialization to vec is infallible")
    }
}

// Since `HexString` is serialized
// exactly like `Vec<u8>`, we can just reuse the standard impl
impl EncodeLike<[u8], HexString> for BorshCodec {
    fn encode_like(&self, borrowed: &[u8]) -> Vec<u8> {
        borsh::to_vec(borrowed).expect("Borsh serialization to vec is infallible")
    }
}
