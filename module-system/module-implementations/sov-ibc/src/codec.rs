use core::fmt::Display;

use borsh::{BorshDeserialize, BorshSerialize};
use ibc::Any;
use ibc_proto::protobuf::{Error, Protobuf};
use sov_state::codec::{BorshCodec, PairOfCodecs, StateKeyCodec, StateValueCodec};

#[derive(Default)]
pub struct ProtobufCodec;

impl<V> StateValueCodec<V> for ProtobufCodec
where
    V: Protobuf<Any>,
    V::Error: Display,
{
    type ValueError = Error;

    fn encode_value(&self, value: &V) -> Vec<u8> {
        value.encode_vec()
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::ValueError> {
        Protobuf::decode_vec(bytes)
    }
}

//FIXME: HACK until PairOfCodecs derives Default
// When implemented, change type declaration back to
// pub type BorshKeyProtobufValueCodec = PairOfCodecs<BorshCodec, ProtobufCodec>;
pub struct BorshKeyProtobufValueCodec(PairOfCodecs<BorshCodec, ProtobufCodec>);

impl<K> StateKeyCodec<K> for BorshKeyProtobufValueCodec
where
    K: BorshSerialize + BorshDeserialize,
{
    type KeyError = <BorshCodec as StateKeyCodec<K>>::KeyError;

    fn encode_key(&self, key: &K) -> Vec<u8> {
        self.0.encode_key(key)
    }

    fn try_decode_key(&self, bytes: &[u8]) -> Result<K, Self::KeyError> {
        self.0.try_decode_key(bytes)
    }
}

impl<V> StateValueCodec<V> for BorshKeyProtobufValueCodec
where
    V: Protobuf<Any>,
    V::Error: Display,
{
    type ValueError = Error;

    fn encode_value(&self, value: &V) -> Vec<u8> {
        self.0.encode_value(value)
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::ValueError> {
        self.0.try_decode_value(bytes)
    }
}

impl Default for BorshKeyProtobufValueCodec {
    fn default() -> Self {
        Self(PairOfCodecs {
            key_codec: BorshCodec::default(),
            value_codec: ProtobufCodec::default(),
        })
    }
}
