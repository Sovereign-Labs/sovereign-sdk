use core::fmt::Display;
use core::marker::PhantomData;

use ibc_proto::protobuf::{Error, Protobuf};
use sov_state::codec::{StateKeyCodec, StateValueCodec};

// TODO:
// We're currently blocked on using this, because the `StateMap` prevents us from using a different codec for key and value. Our keys are never protobuf-encodable (e.g. ClientId); so we'd like to borsh-encode the key, and protobuf-encode the value.
pub struct ProtobufCodec<Raw> {
    _raw: PhantomData<Raw>,
}

impl<K, Raw> StateKeyCodec<K> for ProtobufCodec<Raw>
where
    K: Protobuf<Raw>,
    K::Error: Display,
    Raw: prost::Message + Default,
{
    type KeyError = Error;

    fn encode_key(&self, key: &K) -> Vec<u8> {
        key.encode_vec()
    }

    fn try_decode_key(&self, bytes: &[u8]) -> Result<K, Self::KeyError> {
        Protobuf::decode_vec(bytes)
    }
}

impl<K, Raw> StateValueCodec<K> for ProtobufCodec<Raw>
where
    K: Protobuf<Raw>,
    K::Error: Display,
    Raw: prost::Message + Default,
{
    type ValueError = Error;

    fn encode_value(&self, value: &K) -> Vec<u8> {
        value.encode_vec()
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<K, Self::ValueError> {
        Protobuf::decode_vec(bytes)
    }
}
