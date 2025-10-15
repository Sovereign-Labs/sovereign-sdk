mod client;
#[cfg(test)]
mod tests;

use std::fmt::Debug;
use std::time::Duration;

use async_trait::async_trait;
use celestia_types::nmt::Namespace;
use sov_rollup_interface::da::{DaProof, DaSpec, RelevantBlobs, RelevantProofs};
use sov_rollup_interface::node::da::{DaService, MaybeRetryable, SubmitBlobReceipt};
use tokio::sync::oneshot;
use tracing::instrument;

pub use crate::config::CelestiaConfig;
pub use crate::da_service::client::standard_node::StandardNodeClient;
pub use crate::da_service::client::twinkle::TwinkleClient;
pub use crate::da_service::client::CelestiaClient;
use crate::types::{
    BlobWithSender, FilteredCelestiaBlock, NamespaceBoundaryProof, RollupNamespace,
};
use crate::verifier::proofs::{self, BlobProof};
use crate::verifier::{CelestiaSpec, CelestiaVerifier, RollupParams};

type BoxError = anyhow::Error;

#[derive(Debug, Clone)]
pub struct CelestiaService {
    client: CelestiaClient,
    rollup_batch_namespace: RollupNamespace,
    rollup_proof_namespace: RollupNamespace,
    safe_lead_time: Duration,
}

impl CelestiaService {
    fn with_client(
        submit_client: CelestiaClient,
        rollup_batch_namespace: Namespace,
        rollup_proof_namespace: Namespace,
        safe_lead_time: Duration,
    ) -> Self {
        Self {
            client: submit_client,
            rollup_batch_namespace: RollupNamespace::Batch(rollup_batch_namespace),
            rollup_proof_namespace: RollupNamespace::Proof(rollup_proof_namespace),
            safe_lead_time,
        }
    }
}

impl CelestiaService {
    pub async fn new(config: CelestiaConfig, chain_params: RollupParams) -> Self {
        let submit_client = config.construct_celestia_client().await;

        Self::with_client(
            submit_client,
            chain_params.rollup_batch_namespace,
            chain_params.rollup_proof_namespace,
            Duration::from_millis(config.safe_lead_time_ms),
        )
    }
}

fn into_transient_with_context(
    error: jsonrpsee::core::ClientError,
) -> MaybeRetryable<anyhow::Error> {
    let error = anyhow::anyhow!("Celestia RPC node returned an error: {:?}", error);
    MaybeRetryable::Transient(error)
}

#[async_trait]
impl DaService for CelestiaService {
    type Spec = CelestiaSpec;
    type Config = CelestiaConfig;
    type Verifier = CelestiaVerifier;
    type FilteredBlock = FilteredCelestiaBlock;
    type Error = BoxError;

    #[instrument(skip(self))]
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        self.client
            .get_block_at(
                height,
                &self.rollup_batch_namespace,
                &self.rollup_proof_namespace,
            )
            .await
    }

    #[instrument(skip(self))]
    async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        self.client.get_block_header_at(height).await
    }

    fn safe_lead_time(&self) -> Duration {
        self.safe_lead_time
    }

    #[instrument(skip(self))]
    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        // Tendermint has instant finality, so the head block is the one that finalized
        // and network is always guaranteed to be secure,
        // it can work even if the node is still catching up.
        self.get_head_block_header().await
    }

    #[instrument(skip(self))]
    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        self.client.get_head_block_header().await
    }

    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction> {
        extract_relevant_blobs(block)
    }

    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
    ) -> RelevantProofs<
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    > {
        get_extraction_proof(block, blobs)
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        self.client
            .submit_blob_to_namespace(blob, self.rollup_batch_namespace)
            .await
    }

    async fn send_proof(
        &self,
        aggregated_proof: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        self.client
            .submit_blob_to_namespace(aggregated_proof, self.rollup_proof_namespace)
            .await
    }

    #[instrument(err)]
    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        self.client
            .get_blobs_at(height, &self.rollup_proof_namespace)
            .await
    }

    async fn get_signer(&self) -> <Self::Spec as DaSpec>::Address {
        self.client.get_signer()
    }
}

pub(crate) fn extract_relevant_blobs(
    block: &FilteredCelestiaBlock,
) -> RelevantBlobs<BlobWithSender> {
    let proof_blobs = block.rollup_proof_data.get_blobs_with_sender();
    let batch_blobs = block.rollup_batch_data.get_blobs_with_sender();
    RelevantBlobs {
        proof_blobs,
        batch_blobs,
    }
}

pub(crate) fn get_extraction_proof(
    block: &FilteredCelestiaBlock,
    blobs: &RelevantBlobs<BlobWithSender>,
) -> RelevantProofs<Vec<BlobProof>, Option<NamespaceBoundaryProof>> {
    let batch = {
        let inclusion_proof = proofs::new_inclusion_proof(
            &block.header,
            &block.rollup_batch_data,
            &blobs.batch_blobs,
        );

        DaProof {
            inclusion_proof,
            completeness_proof: NamespaceBoundaryProof::from_namespace_data(
                &block.rollup_batch_data,
            ),
        }
    };

    let proof = {
        // Note: The second call to new_inclusion_proof merklizes and parse the executable transactions namespace again.
        let inclusion_proof = proofs::new_inclusion_proof(
            &block.header,
            &block.rollup_proof_data,
            &blobs.proof_blobs,
        );

        DaProof {
            inclusion_proof,
            completeness_proof: NamespaceBoundaryProof::from_namespace_data(
                &block.rollup_proof_data,
            ),
        }
    };

    RelevantProofs { proof, batch }
}
