use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use sha2::Digest;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::maybestd::sync::Arc;
use sov_rollup_interface::services::da::{DaService, SlotData};
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio::time;

use crate::types::{MockAddress, MockBlob, MockBlock, MockDaVerifier};
use crate::verifier::MockDaSpec;
use crate::{MockBlockHeader, MockHash};

#[derive(Clone)]
/// DaService used in tests.
/// Currently only supports single blob per block.
/// Finalized blocks are removed after being read, except last one
/// Height of the first submitted block is 0
pub struct MockDaService {
    sequencer_da_address: MockAddress,
    blocks: Arc<RwLock<VecDeque<MockBlock>>>,
    blocks_to_finality: u32,
    /// Used for calculating correct finality from state of `blocks`
    last_finalized_height: Arc<AtomicU64>,
    finalized_header_sender: Option<Sender<MockBlockHeader>>,
}

impl MockDaService {
    /// Creates a new [`MockDaService`] with instant finality.
    pub fn new(sequencer_da_address: MockAddress) -> Self {
        Self::with_finality(sequencer_da_address, 0)
    }

    /// Create a new [`MockDaService`] with given finality.
    pub fn with_finality(sequencer_da_address: MockAddress, blocks_to_finality: u32) -> Self {
        Self {
            sequencer_da_address,
            blocks: Arc::new(Default::default()),
            blocks_to_finality,
            last_finalized_height: Arc::new(AtomicU64::new(0)),
            finalized_header_sender: None,
        }
    }

    async fn wait_for_height(&self, height: u64) {
        // Waits for 100 seconds blob to be submitted
        for _ in 0..100_000 {
            {
                if self
                    .blocks
                    .read()
                    .await
                    .iter()
                    .any(|b| b.header().height() == height)
                {
                    return;
                }
            }
            time::sleep(Duration::from_millis(10)).await;
        }
        panic!("No blob at {height} has been sent in time");
    }
}

#[async_trait]
impl DaService for MockDaService {
    type Spec = MockDaSpec;
    type Verifier = MockDaVerifier;
    type FilteredBlock = MockBlock;
    type Error = anyhow::Error;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        // Block until there's something
        self.wait_for_height(height).await;
        // Locking blocks here, so submissions has to wait
        let mut blocks = self.blocks.write().await;
        let oldest_available_height = blocks[0].header.height;
        let index = height
            .checked_sub(oldest_available_height)
            .ok_or(anyhow::anyhow!(
                "Block at height {} is not available anymore",
                height
            ))?;

        // We still return error, as it is possible, that block has been consumed between `wait` and locking blocks
        let block = blocks
            .get(index as usize)
            .ok_or(anyhow::anyhow!(
                "Block at height {} is not available anymore",
                height
            ))?
            .clone();

        // Block that preceeds last finalized block is evicted at first read.
        // Caller can always get last finalized block, or read everything if it is called in order
        // If readers are from multiple threads, then block will be lost.
        // This is optimization for long-running cases
        // Maybe simply storing all blocks is fine, all only keep 100 last finalized.
        let last_finalized_height = self.last_finalized_height.load(Ordering::Acquire);
        if last_finalized_height > 0 && oldest_available_height < (last_finalized_height - 1) {
            blocks.pop_front();
        }

        Ok(block)
    }

    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let blocks = self.blocks.read().await;
        if blocks.len() < self.blocks_to_finality as usize + 1 {
            anyhow::bail!("MockChain hasn't progressed enough to finalize");
        }

        let oldest_available_height = blocks[0].header().height();
        let last_finalized_height = self.last_finalized_height.load(Ordering::Acquire);

        let index = last_finalized_height
            .checked_sub(oldest_available_height)
            .expect("Inconsistent MockDa");

        Ok(*blocks[index as usize].header())
    }

    fn subscribe_finalized_header(
        &mut self,
    ) -> Result<Receiver<<Self::Spec as DaSpec>::BlockHeader>, Self::Error> {
        if let Some(sender) = &self.finalized_header_sender {
            return Ok(sender.subscribe());
        }
        let (s, rx) = tokio::sync::broadcast::channel(100);
        self.finalized_header_sender = Some(s);
        Ok(rx)
    }

    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let blocks = self.blocks.read().await;

        blocks
            .iter()
            .last()
            .map(|b| *b.header())
            .ok_or(anyhow::anyhow!("MockChain is empty"))
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
        let mut blocks = self.blocks.write().await;

        let (previous_block_hash, height) = match blocks.iter().last().map(|b| *b.header()) {
            None => (MockHash::from([0; 32]), 0),
            Some(block_header) => (block_header.hash(), block_header.height + 1),
        };
        let data_hash = hash_to_array(blob);
        // Hash only from single blob
        let block_hash = block_hash(height, data_hash, previous_block_hash.into());

        let blob = MockBlob::new(blob.to_vec(), self.sequencer_da_address, data_hash);
        let header = MockBlockHeader {
            prev_hash: previous_block_hash,
            hash: block_hash,
            height,
        };
        let block = MockBlock {
            header,
            validity_cond: Default::default(),
            blobs: vec![blob],
        };
        blocks.push_back(block);

        // Enough blocks to finalize block
        if blocks.len() > self.blocks_to_finality as usize {
            let oldest_available_height = blocks[0].header().height();
            let last_finalized_height = self.last_finalized_height.load(Ordering::Acquire);

            let last_finalized_index = last_finalized_height
                .checked_sub(oldest_available_height)
                .unwrap();
            let next_index_to_finalize = blocks.len() - self.blocks_to_finality as usize - 1;
            assert_eq!(next_index_to_finalize as u64, last_finalized_index + 1);

            if let Some(finalized_header_sender) = &self.finalized_header_sender {
                finalized_header_sender
                    .send(*blocks[next_index_to_finalize].header())
                    .unwrap();
            }

            let this_finalized_height = oldest_available_height + next_index_to_finalize as u64;
            self.last_finalized_height
                .store(this_finalized_height, Ordering::Release);
        }

        Ok(())
    }
}

fn hash_to_array(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    result
        .as_slice()
        .try_into()
        .expect("SHA256 should be 32 bytes")
}

fn block_hash(height: u64, data_hash: [u8; 32], prev_hash: [u8; 32]) -> MockHash {
    let mut block_to_hash = height.to_be_bytes().to_vec();
    block_to_hash.extend_from_slice(&data_hash[..]);
    block_to_hash.extend_from_slice(&prev_hash[..]);

    MockHash::from(hash_to_array(&block_to_hash))
}

#[cfg(test)]
mod tests {
    use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait};
    use tokio::task::JoinHandle;

    use super::*;

    #[tokio::test]
    async fn test_empty() {
        let da = MockDaService::new(MockAddress::new([1; 32]));

        let last_finalized_block_response = da.get_last_finalized_block_header().await;
        assert!(last_finalized_block_response.is_err());
        assert_eq!(
            "MockChain hasn't progressed enough to finalize",
            last_finalized_block_response.err().unwrap().to_string()
        );
        let head_block_header_response = da.get_head_block_header().await;
        assert!(head_block_header_response.is_err());
        assert_eq!(
            "MockChain is empty",
            head_block_header_response.err().unwrap().to_string()
        );
    }

    fn get_finalized_headers_collector(da: &mut MockDaService) -> JoinHandle<Vec<MockBlockHeader>> {
        let mut receiver = da.subscribe_finalized_header().unwrap();
        tokio::spawn(async move {
            let mut received = Vec::with_capacity(10);
            // Read until it's empty
            while let Ok(header) = receiver.try_recv() {
                received.push(header);
            }
            received
        })
    }

    // Checks that last finalized height is always less than last submitted by blocks_to_finalization
    fn validate_get_finalized_header_response(
        submit_height: u64,
        blocks_to_finalization: u64,
        response: anyhow::Result<MockBlockHeader>,
    ) {
        if let Some(expected_finalized_height) = submit_height.checked_sub(blocks_to_finalization) {
            assert_eq!(expected_finalized_height, response.unwrap().height());
        } else {
            assert!(response.is_err());
            assert_eq!(
                "MockChain hasn't progressed enough to finalize",
                response.err().unwrap().to_string()
            );
        }
    }

    async fn test_push_and_read(finalization: u64, num_blocks: usize) {
        let mut da = MockDaService::with_finality(MockAddress::new([1; 32]), finalization as u32);
        let collector_handle = get_finalized_headers_collector(&mut da);

        for i in 0..=num_blocks {
            let published_blob: Vec<u8> = vec![i as u8; i + 1];
            let i = i as u64;

            da.send_transaction(&published_blob).await.unwrap();

            let mut block = da.get_block_at(i).await.unwrap();

            assert_eq!(i, block.header.height());
            assert_eq!(1, block.blobs.len());
            let blob = &mut block.blobs[0];
            let retrieved_data = blob.full_data().to_vec();
            assert_eq!(published_blob, retrieved_data);

            let last_finalized_block_response = da.get_last_finalized_block_header().await;
            validate_get_finalized_header_response(i, finalization, last_finalized_block_response);
        }

        let received = collector_handle.await.unwrap();
        let heights: Vec<u64> = received.iter().map(|h| h.height()).collect();
        let expected_heights: Vec<u64> = (0..=(num_blocks as u64 - finalization)).collect();
        assert_eq!(expected_heights, heights);
    }

    async fn test_push_many_then_read(finalization: u64, num_blocks: usize) {
        let mut da = MockDaService::with_finality(MockAddress::new([1; 32]), finalization as u32);
        let collector_handle = get_finalized_headers_collector(&mut da);

        let blobs: Vec<Vec<u8>> = (0..=num_blocks).map(|i| vec![i as u8; i + 1]).collect();

        // Submitting blobs first
        for (i, blob) in blobs.iter().enumerate() {
            let i = i as u64;
            // Send transaction should pass
            da.send_transaction(blob).await.unwrap();
            let last_finalized_block_response = da.get_last_finalized_block_header().await;
            validate_get_finalized_header_response(i, finalization, last_finalized_block_response);

            let head_block_header = da.get_head_block_header().await.unwrap();
            assert_eq!(i, head_block_header.height());
        }

        let expected_finalized_height = blobs.len() as u64 - finalization - 1;

        // Then read
        for (i, blob) in blobs.into_iter().enumerate() {
            let i = i as u64;

            let mut fetched_block = da.get_block_at(i).await.unwrap();
            assert_eq!(i, fetched_block.header().height());

            let last_finalized_header = da.get_last_finalized_block_header().await.unwrap();
            assert_eq!(expected_finalized_height, last_finalized_header.height());

            assert_eq!(&blob, fetched_block.blobs[0].full_data());

            let head_block_header = da.get_head_block_header().await.unwrap();
            assert_eq!(num_blocks, head_block_header.height() as usize);
        }

        let received = collector_handle.await.unwrap();
        let heights: Vec<u64> = received.iter().map(|h| h.height()).collect();
        let expected_heights: Vec<u64> = (0..=(num_blocks as u64 - finalization)).collect();
        assert_eq!(expected_heights, heights);
    }

    mod instant_finality {
        use super::*;
        #[tokio::test]
        /// Pushing a blob and immediately reading it
        async fn push_pull_single_thread() {
            test_push_and_read(0, 10).await;
        }

        #[tokio::test]
        async fn push_many_then_read() {
            test_push_many_then_read(0, 10).await;
        }
    }

    mod non_instant_finality {
        use super::*;

        #[tokio::test]
        async fn push_pull_single_thread() {
            test_push_and_read(1, 10).await;
            test_push_and_read(3, 10).await;
            test_push_and_read(5, 10).await;
        }

        #[tokio::test]
        async fn push_many_then_read() {
            test_push_many_then_read(1, 10).await;
            test_push_many_then_read(3, 10).await;
            test_push_many_then_read(5, 10).await;
        }
    }
}
