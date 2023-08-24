use crate::codec::StateValueCodec;

/// A [`StateValueCodec`] that uses [`borsh`] for all values.
#[derive(Debug, Default, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct BorshCodec;

impl<V> StateValueCodec<V> for BorshCodec
where
    V: borsh::BorshSerialize + borsh::BorshDeserialize,
{
    type Error = std::io::Error;

    fn encode_value(&self, value: &V) -> Vec<u8> {
        value.try_to_vec().expect("Failed to serialize value")
    }

    fn try_decode_value(&self, bytes: &[u8]) -> Result<V, Self::Error> {
        V::try_from_slice(bytes)
    }
}
