use primitive_types::H256;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::BlockHashTrait;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AvailHash(H256);

impl AvailHash {
    pub fn new(hash: H256) -> Self {
        Self(hash)
    }
}

impl BlockHashTrait for AvailHash {}

impl core::fmt::Display for AvailHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

impl AsRef<[u8]> for AvailHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<AvailHash> for [u8; 32] {
    fn from(value: AvailHash) -> Self {
        value.0.to_fixed_bytes()
    }
}

impl AvailHash {
    pub fn inner(&self) -> &[u8; 32] {
        self.0.as_fixed_bytes()
    }
}
