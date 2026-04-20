use serde::{Deserialize, Serialize};

use super::CodeCommitmentHash;
use crate::da::DaSpec;

/// A single deferred proof input containing its public values and associated DA block header.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "Da::BlockHeader: Serialize",
    deserialize = "Da::BlockHeader: Deserialize<'de>"
))]
pub struct DeferredProofInput<Da: DaSpec> {
    /// The public values of the proof.
    pub public_values: Vec<u8>,
    /// The DA block header associated with this proof.
    pub da_block_header: Da::BlockHeader,
}

/// Witness data for an aggregated proof, containing the proof inputs and verification metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "DeferredProofInput<Da>: Serialize",
    deserialize = "DeferredProofInput<Da>: Deserialize<'de>"
))]
pub struct AggregatedProofWitness<Da: DaSpec> {
    /// The inner proof inputs to be aggregated.
    pub proof_inputs: Vec<DeferredProofInput<Da>>,
    /// The hash of the inner verification key used to verify the aggregated proofs.
    pub inner_vkey_hash: CodeCommitmentHash,
    /// The hash of the outer verification key.
    pub outer_vkey_hash: CodeCommitmentHash,
    /// An optional previous outer proof witness for recursive aggregation.
    pub prev_outer_proof_witness: Option<PreviousOuterProofWitness>,
}

/// Contains the serialized public values of a previous outer aggregation proof.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviousOuterProofWitness {
    /// Serialized public values of the previous proof.
    pub public_values: Vec<u8>,
}
