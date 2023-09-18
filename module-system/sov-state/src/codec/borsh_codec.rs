use super::{StateCodec, StateKeyCodec};
use crate::codec::StateValueCodec;

/// A [`StateCodec`] that uses [`borsh`] for all keys and values.
#[derive(Debug, Default, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct BorshCodec;

impl<K> StateKeyCodec<K> for BorshCodec
where
    K: borsh::BorshSerialize + borsh::BorshDeserialize,
{
    fn encode_key(&self, value: &K) -> Vec<u8> {
        value.try_to_vec().expect("Failed to serialize key")
    }
}

impl<V> StateValueCodec<V> for BorshCodec
where
    V: borsh::BorshSerialize + borsh::BorshDeserialize,
{
    type Error = std::io::Error;

    fn encode_value(&self, value: &V) -> Vec<u8> {
        value.try_to_vec().expect("Failed to serialize value")
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::Error> {
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
