use core::fmt::Display;

use ibc::Any;
use ibc_proto::protobuf::{Error, Protobuf};
use sov_state::codec::{BorshCodec, PairOfCodecs, StateValueCodec};

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

pub type BorshKeyProtobufValueCodec = PairOfCodecs<BorshCodec, ProtobufCodec>;
