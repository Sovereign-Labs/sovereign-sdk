use serde::{Deserialize, Serialize};
use sov_modules_api::DaSpec;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeferredProofInput<Da: DaSpec> {
    pub public_values: Vec<u8>,
    pub da_block_header: Da::BlockHeader,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregatedProofWitness<Da: DaSpec> {
    pub proof_inputs: Vec<DeferredProofInput<Da>>,
    pub vkey_hash: [u32; 8],
    pub prev_outer_proof_witness: Option<PreviousOuterProofWitness>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviousOuterProofWitness {
    pub public_values: Vec<u8>,
    pub vkey_hash: [u32; 8],
}
