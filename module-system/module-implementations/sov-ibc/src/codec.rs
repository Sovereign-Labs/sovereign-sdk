use core::fmt::Display;

use ibc::Any;
use ibc_proto::protobuf::{Error, Protobuf};
use sov_state::codec::StateValueCodec;

pub struct ProtobufCodec;

impl<K> StateValueCodec<K> for ProtobufCodec
where
    K: Protobuf<Any>,
    K::Error: Display,
{
    type ValueError = Error;

    fn encode_value(&self, value: &K) -> Vec<u8> {
        value.encode_vec()
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<K, Self::ValueError> {
        Protobuf::decode_vec(bytes)
    }
}
