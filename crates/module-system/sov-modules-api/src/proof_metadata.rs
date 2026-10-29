use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::optimistic::{SerializedAttestation, SerializedChallenge};
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;

use crate::transaction::TxDetails;
use crate::Spec;

/// Proof type supported by the rollup.

#[derive(
    Debug,
    PartialEq,
    Eq,
    Clone,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ProofType {
    /// ZK workflow: aggregated zk proof.
    ZkAggregatedProof(SerializedAggregatedProof),
    /// Optimistic workflow: attestation.
    OptimisticProofAttestation(SerializedAttestation),
    /// Optimistic workflow: challenge.
    OptimisticProofChallenge(SerializedChallenge, SlotNumber),
}

/// Proof with metadata need for verification.
#[derive(Debug, PartialEq, Eq, Clone, borsh::BorshDeserialize)]
#[cfg_attr(
    feature = "native",
    derive(borsh::BorshSerialize, serde::Serialize, serde::Deserialize,)
)]
#[cfg_attr(feature = "native", serde(bound = "S: Spec"))]
pub struct SerializeProofWithDetails<S: Spec> {
    /// The serialized aggregated proof.
    pub proof: ProofType,
    /// The transaction metadata.
    pub details: TxDetails<S>,
}
