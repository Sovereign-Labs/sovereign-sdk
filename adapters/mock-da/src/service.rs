use async_trait::async_trait;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::maybestd::sync::Arc;
use sov_rollup_interface::services::da::DaService;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::Mutex;

use crate::types::{MockAddress, MockBlob, MockBlock, MockDaSpec, MockDaVerifier};

#[derive(Clone)]
/// DaService used in tests.
pub struct MockDaService {
    sender: Sender<Vec<u8>>,
    receiver: Arc<Mutex<Receiver<Vec<u8>>>>,
    sequencer_da_address: MockAddress,
}

impl MockDaService {
    /// Creates a new MockDaService.
    pub fn new(sequencer_da_address: MockAddress) -> Self {
        let (sender, receiver) = mpsc::channel(100);
        Self {
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
            sequencer_da_address,
        }
    }
}

#[async_trait]
impl DaService for MockDaService {
    type Spec = MockDaSpec;
    type Verifier = MockDaVerifier;
    type FilteredBlock = MockBlock;
    type Error = anyhow::Error;

    async fn get_finalized_at(&self, _height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let data = self.receiver.lock().await.recv().await;
        let data = data.unwrap();
        let hash = [0; 32];

        let blob = MockBlob::new(data, self.sequencer_da_address, hash);

        Ok(MockBlock {
            blobs: vec![blob],
            ..Default::default()
        })
    }

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        self.get_finalized_at(height).await
    }

    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> Vec<<Self::Spec as DaSpec>::BlobTransaction> {
        block.blobs.clone()
    }

    async fn get_extraction_proof(
        &self,
        _block: &Self::FilteredBlock,
        _blobs: &[<Self::Spec as DaSpec>::BlobTransaction],
    ) -> (
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    ) {
        ([0u8; 32], ())
    }

    async fn send_transaction(&self, blob: &[u8]) -> Result<(), Self::Error> {
        self.sender.send(blob.to_vec()).await.unwrap();
        Ok(())
    }
}
