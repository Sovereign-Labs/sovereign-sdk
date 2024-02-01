use serde_json;

use super::{StateCodec, StateKeyCodec};
use crate::codec::StateValueCodec;

/// A [`StateCodec`] that uses [`serde_json`] for all keys and values.
#[derive(Debug, Default, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
pub struct JsonCodec;

impl<K> StateKeyCodec<K> for JsonCodec
where
    K: serde::Serialize,
{
    fn encode_key(&self, key: &K) -> Vec<u8> {
        serde_json::to_vec(key).expect("Failed to serialize key")
    }
}

impl<V> StateValueCodec<V> for JsonCodec
where
    V: serde::Serialize + for<'a> serde::Deserialize<'a>,
{
    type Error = serde_json::Error;

    fn encode_value(&self, value: &V) -> Vec<u8> {
        serde_json::to_vec(value).expect("Failed to serialize value")
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::Error> {
        serde_json::from_slice(bytes)
    }
}

impl StateCodec for JsonCodec {
    type KeyCodec = Self;
    type ValueCodec = Self;

    fn key_codec(&self) -> &Self::KeyCodec {
        self
    }

    fn value_codec(&self) -> &Self::ValueCodec {
        self
    }
}
