use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::BlockHashTrait;
use subxt::utils::H256;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct AvailHash(pub H256);

impl BlockHashTrait for AvailHash {}

impl AsRef<[u8]> for AvailHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}
