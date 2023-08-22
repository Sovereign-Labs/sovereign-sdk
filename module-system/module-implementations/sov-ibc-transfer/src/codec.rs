use std::convert::Infallible;

use sov_state::codec::{BorshCodec, PairOfCodecs, StateKeyCodec};

#[derive(Clone, Default)]
pub struct RawKeyCodec;

impl<K> StateKeyCodec<K> for RawKeyCodec
where
    K: AsRef<[u8]> + FromIterator<u8>,
{
    type KeyError = Infallible;

    fn encode_key(&self, key: &K) -> Vec<u8> {
        key.as_ref().to_vec()
    }

    fn try_decode_key(&self, bytes: &[u8]) -> Result<K, Self::KeyError> {
        Ok(K::from_iter(bytes.to_vec()))
    }
}

/// This codec leaves the key untouched, and borsh-serializes values.
/// Hence, the key is forced to be
pub type RawKeyBorshValueCodec = PairOfCodecs<RawKeyCodec, BorshCodec>;
