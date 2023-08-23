use crate::codec::{StateKeyCodec, StateValueCodec};

/// A [`StateCodec`] that uses [`borsh`] for all keys and values.
#[derive(Debug, Default, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct BincodeCodec;

impl<K> StateKeyCodec<K> for BincodeCodec
where
    K: serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    type KeyError = bincode::Error;

    fn encode_key(&self, key: &K) -> Vec<u8> {
        bincode::serialize(key).expect("Failed to serialize key")
    }

    fn try_decode_key(&self, bytes: &[u8]) -> Result<K, Self::KeyError> {
        bincode::deserialize(bytes)
    }
}

impl<V> StateValueCodec<V> for BincodeCodec
where
    V: serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    type ValueError = bincode::Error;

    fn encode_value(&self, value: &V) -> Vec<u8> {
        bincode::serialize(value).expect("Failed to serialize key")
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::ValueError> {
        bincode::deserialize(bytes)
    }
}
