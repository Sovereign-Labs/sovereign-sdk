use async_trait::async_trait;

use crate::common::SlotNumber;
use crate::optimistic::{SerializedAttestation, SerializedChallenge};
use crate::zk::aggregated_proof::SerializedAggregatedProof;

/// Snapshot of a blob sender's in-flight count against its configured
/// `max_concurrent` cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobSenderStatus {
    /// Number of blobs currently in flight on the DA layer.
    pub in_flight: usize,
    /// The configured maximum number of concurrent in-flight blobs.
    pub max_concurrent: usize,
}

impl BlobSenderStatus {
    /// Returns `true` if the number of blobs in flight meets or exceeds the
    /// configured `max_concurrent` cap.
    pub fn is_busy(&self) -> bool {
        self.in_flight >= self.max_concurrent
    }
}

impl std::fmt::Display for BlobSenderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} blobs in flight, max_concurrent={}",
            self.in_flight, self.max_concurrent
        )
    }
}

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

    /// Returns a [`BlobSenderStatus`] snapshot of in-flight proof blobs
    /// against the configured `max_concurrent_proof_blobs` cap.
    async fn proof_blob_sender_status(&self) -> anyhow::Result<BlobSenderStatus>;
}
