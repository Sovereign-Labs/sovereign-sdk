use crate::codec::{StateKeyCodec, StateValueCodec};

/// A [`StateCodec`] that uses [`borsh`] for all keys and values.
#[derive(Debug, Default, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct BorshCodec;

impl<K> StateKeyCodec<K> for BorshCodec
where
    K: borsh::BorshSerialize + borsh::BorshDeserialize,
{
    type KeyError = std::io::Error;

    fn encode_key(&self, key: &K) -> Vec<u8> {
        key.try_to_vec().expect("Failed to serialize key")
    }

    fn try_decode_key(&self, bytes: &[u8]) -> Result<K, Self::KeyError> {
        K::try_from_slice(bytes)
    }
}

impl<V> StateValueCodec<V> for BorshCodec
where
    V: borsh::BorshSerialize + borsh::BorshDeserialize,
{
    type ValueError = std::io::Error;

    fn encode_value(&self, value: &V) -> Vec<u8> {
        value.try_to_vec().expect("Failed to serialize value")
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::ValueError> {
        V::try_from_slice(bytes)
    }
}
