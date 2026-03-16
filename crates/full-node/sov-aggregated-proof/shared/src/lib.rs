use serde::{Deserialize, Serialize};
use sov_mock_da::MockBlockHeader;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeferredProofInput {
    pub public_values: Vec<u8>,
    pub vkey_hash: [u32; 8],
    pub da_block_header: MockBlockHeader,
}
