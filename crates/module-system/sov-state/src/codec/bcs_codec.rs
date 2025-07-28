use sov_rollup_interface::common::HexString;

use super::{EncodeLike, StateCodec, StateItemDecoder, StateItemEncoder};

/// A [`StateCodec`] that uses [`bcs`] for all keys and values.
#[derive(Debug, Default, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
pub struct BcsCodec;

impl<V> StateItemEncoder<V> for BcsCodec
where
    V: serde::Serialize,
{
    fn encode(&self, value: &V) -> Vec<u8> {
        bcs::to_bytes(value).expect("Failed to serialize value")
    }
}

impl<V> StateItemDecoder<V> for BcsCodec
where
    V: for<'a> serde::Deserialize<'a>,
{
    type Error = bcs::Error;

    fn try_decode(&self, bytes: &[u8]) -> Result<V, Self::Error> {
        bcs::from_bytes(bytes)
    }
}

impl StateCodec for BcsCodec {
    type KeyCodec = Self;
    type ValueCodec = Self;

    fn key_codec(&self) -> &Self::KeyCodec {
        self
    }

    fn value_codec(&self) -> &Self::ValueCodec {
        self
    }
}

// [`bcs`] serializes slices and vectors the same way, i.e. by calling
// [`serde::Serializer::collect_seq`] under the hood.
impl<T: serde::Serialize> EncodeLike<[T], Vec<T>> for BcsCodec {
    fn encode_like(&self, borrowed: &[T]) -> Vec<u8> {
        bcs::to_bytes(borrowed).expect("Bcs serialization to vec is infallible")
    }
}

impl EncodeLike<[u8], HexString> for BcsCodec {
    fn encode_like(&self, borrowed: &[u8]) -> Vec<u8> {
        bcs::to_bytes(borrowed).expect("Bcs serialization to vec is infallible")
    }
}
