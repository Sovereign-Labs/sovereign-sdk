//! This module defines a codec which delegates to one codec for keys and one codec for values.

use super::{StateKeyCodec, StateValueCodec};

/// A [`StateValueCodec`] that uses [`serde_json`] for all values.
#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct SplitCodec<KC, VC> {
    pub key_codec: KC,
    pub value_codec: VC,
}

impl<K, KC, VC> StateKeyCodec<K> for SplitCodec<KC, VC>
where
    KC: StateKeyCodec<K>,
{
    fn encode_key(&self, key: &K) -> Vec<u8> {
        self.key_codec.encode_key(key)
    }
}

impl<V, KC, VC> StateValueCodec<V> for SplitCodec<KC, VC>
where
    VC: StateValueCodec<V>,
{
    type Error = VC::Error;

    fn encode_value(&self, value: &V) -> Vec<u8> {
        self.value_codec.encode_value(value)
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::Error> {
        self.value_codec.try_decode_value(bytes)
    }
}
