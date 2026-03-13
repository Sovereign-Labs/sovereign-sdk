use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredProofInput {
    pub public_values: Vec<u8>,
    pub vkey_hash: [u32; 8],
}
