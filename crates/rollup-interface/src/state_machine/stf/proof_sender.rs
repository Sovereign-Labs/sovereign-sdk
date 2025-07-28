use async_trait::async_trait;

use crate::common::SlotNumber;
use crate::optimistic::{SerializedAttestation, SerializedChallenge};
use crate::zk::aggregated_proof::SerializedAggregatedProof;

/// Publishes proof blobs and adds metadata needed for verification.
#[async_trait]
pub trait ProofSender: Send + Sync {
    /// Creates a proof blob with metadata needed for verification.
    async fn publish_proof_blob_with_metadata(
        &self,
        serialized_proof: SerializedAggregatedProof,
    ) -> anyhow::Result<()>;

    /// Creates an attestation blob with metadata needed for verification.
    async fn publish_attestation_blob_with_metadata(
        &self,
        serialized_attestation: SerializedAttestation,
    ) -> anyhow::Result<()>;

    /// Creates a challenge blob with metadata needed for verification.
    async fn publish_challenge_blob_with_metadata(
        &self,
        serialized_challenge: SerializedChallenge,
        slot_height: SlotNumber,
    ) -> anyhow::Result<()>;
}
