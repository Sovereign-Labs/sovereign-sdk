use primitive_types::H256;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::BlockHashTrait;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct AvailHash(pub H256);

impl BlockHashTrait for AvailHash {}

impl AsRef<[u8]> for AvailHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl AvailHash {
    pub fn inner(&self) -> &[u8; 32] {
        self.0.as_fixed_bytes()
    }
}
