//! Data Availability service is a controller of [`StorableMockDaLayer`].

use async_trait::async_trait;
use sov_rollup_interface::da::{DaSpec, RelevantBlobs, RelevantProofs};
use sov_rollup_interface::node::da::{DaService, SubmitBlobReceipt};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::storable::StorableMockDaService;

use crate::{
    BlockProducingConfig, MockBlock, MockDaConfig, MockDaSpec, MockDaVerifier,
    DEFAULT_BLOCK_WAITING_TIME_MS,
};

#[async_trait]
impl DaService for StorableMockDaService {
    type Spec = MockDaSpec;
    type Config = MockDaConfig;
    type Verifier = MockDaVerifier;
    type FilteredBlock = MockBlock;
    type Error = anyhow::Error;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        self.get_block_at_inner(height).await
    }

    async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        self.get_block_header_at_inner(height).await
    }

    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        self.get_last_finalized_block_header().await
    }

    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let head_block_header = { self.head_block.borrow().clone() };
        Ok(head_block_header)
    }

    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction> {
        block.as_relevant_blobs()
    }

    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        _blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
    ) -> RelevantProofs<
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    > {
        block.get_relevant_proofs()
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        let (tx, rx) = oneshot::channel();
        let res = self.send_transaction_inner(blob).await;
        tx.send(res).unwrap();
        rx
    }

    async fn send_proof(
        &self,
        aggregated_proof_data: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        let (tx, rx) = oneshot::channel();
        let res = self.send_proof_inner(aggregated_proof_data).await;
        tx.send(res).unwrap();
        rx
    }

    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        self.get_proofs_at_inner(height).await
    }

    async fn take_background_join_handle(&self) -> Option<JoinHandle<()>> {
        self.block_producer_handle.lock().await.take()
    }

    async fn get_signer(&self) -> Option<<Self::Spec as DaSpec>::Address> {
        Some(self.sequencer_da_address)
    }

    async fn get_approximate_block_time(&self) -> Duration {
        match self.block_producing {
            BlockProducingConfig::Periodic { block_time_ms } => {
                std::time::Duration::from_millis(block_time_ms)
            }
            BlockProducingConfig::OnBatchSubmit {
                block_wait_timeout_ms,
            }
            | BlockProducingConfig::OnAnySubmit {
                block_wait_timeout_ms,
            } => std::time::Duration::from_millis(
                block_wait_timeout_ms.unwrap_or(DEFAULT_BLOCK_WAITING_TIME_MS),
            ),
            BlockProducingConfig::Manual => {
                std::time::Duration::from_secs(DEFAULT_BLOCK_WAITING_TIME_MS)
            }
        }
    }
}
