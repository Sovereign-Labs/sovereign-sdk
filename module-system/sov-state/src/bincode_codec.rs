use crate::codec::{StateKeyCodec, StateValueCodec};

//// TODO
#[derive(Debug, Default, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct BincodeCodec;

impl<K> StateKeyCodec<K> for BincodeCodec
where
    K: serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    type KeyError = bcs::Error;

    fn encode_key(&self, key: &K) -> Vec<u8> {
        bcs::to_bytes(key).expect("Failed to serialize key")
    }

    fn try_decode_key(&self, bytes: &[u8]) -> Result<K, Self::KeyError> {
        bcs::from_bytes(bytes)
    }
}

impl<V> StateValueCodec<V> for BincodeCodec
where
    V: serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    type ValueError = bcs::Error;

    fn encode_value(&self, value: &V) -> Vec<u8> {
        bcs::to_bytes(value).expect("Failed to serialize value")
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::ValueError> {
        bcs::from_bytes(bytes)
    }
}
