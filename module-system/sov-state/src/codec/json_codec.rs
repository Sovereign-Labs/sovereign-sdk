use serde_json;

use crate::codec::StateValueCodec;

/// A [`StateValueCodec`] that uses [`serde_json`] for all values.
#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct JsonCodec;

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
