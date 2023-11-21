use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use pin_project::pin_project;
use sha2::Digest;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::maybestd::sync::Arc;
use sov_rollup_interface::services::da::{DaService, SlotData};
use tokio::sync::{broadcast, RwLock};
use tokio::time;

use crate::types::{MockAddress, MockBlob, MockBlock, MockDaVerifier};
use crate::verifier::MockDaSpec;
use crate::{MockBlockHeader, MockHash};

#[derive(Clone)]
/// DaService used in tests.
/// Currently only supports single blob per block.
/// Finalized blocks are removed after being read, except last one.
/// Height of the first submitted block is 0.
/// It can be used in multithreaded environment with single reader and multiple submitters
/// Multiple consumers produce inconsistent results.
pub struct MockDaService {
    sequencer_da_address: MockAddress,
    blocks: Arc<RwLock<VecDeque<MockBlock>>>,
    /// How many blocks should be submitted, before block is finalized. 0 means instant finality.
    blocks_to_finality: u32,
    /// Used for calculating correct finality from state of `blocks`
    last_finalized_height: Arc<AtomicU64>,
    finalized_header_sender: broadcast::Sender<MockBlockHeader>,
    wait_attempts: usize,
}

impl MockDaService {
    /// Creates a new [`MockDaService`] with instant finality.
    pub fn new(sequencer_da_address: MockAddress) -> Self {
        Self::with_finality(sequencer_da_address, 0)
    }

    /// Create a new [`MockDaService`] with given finality.
    pub fn with_finality(sequencer_da_address: MockAddress, blocks_to_finality: u32) -> Self {
        let (tx, rx1) = broadcast::channel(16);
        // Spawn a task, so channel is never closed
        tokio::spawn(async move {
            let mut rx = rx1;
            while let Ok(header) = rx.recv().await {
                tracing::debug!("Finalized MockHeader: {}", header);
            }
        });
        Self {
            sequencer_da_address,
            blocks: Arc::new(Default::default()),
            blocks_to_finality,
            last_finalized_height: Arc::new(AtomicU64::new(0)),
            finalized_header_sender: tx,
            wait_attempts: 100_0000,
        }
    }

    async fn wait_for_height(&self, height: u64) -> anyhow::Result<()> {
        // Waits self.wait_attempts * 10ms to get finalized header
        for _ in 0..self.wait_attempts {
            {
                if self
                    .blocks
                    .read()
                    .await
                    .iter()
                    .any(|b| b.header().height() == height)
                {
                    return Ok(());
                }
            }
            time::sleep(Duration::from_millis(10)).await;
        }
        anyhow::bail!("No blob at height={height} has been sent in time")
    }
}

#[pin_project]
/// Stream of finalized headers
pub struct MockDaBlockHeaderStream {
    #[pin]
    inner: tokio_stream::wrappers::BroadcastStream<MockBlockHeader>,
}

impl MockDaBlockHeaderStream {
    /// Create new stream of finalized headers
    pub fn new(receiver: broadcast::Receiver<MockBlockHeader>) -> Self {
        Self {
            inner: tokio_stream::wrappers::BroadcastStream::new(receiver),
        }
    }
}

impl futures::Stream for MockDaBlockHeaderStream {
    type Item = Result<MockBlockHeader, anyhow::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project(); // Requires the pin-project crate or similar functionality
        this.inner
            .poll_next(cx)
            .map(|opt| opt.map(|res| res.map_err(Into::into)))
    }
}

#[async_trait]
impl DaService for MockDaService {
    type Spec = MockDaSpec;
    type Verifier = MockDaVerifier;
    type FilteredBlock = MockBlock;
    type HeaderStream = MockDaBlockHeaderStream;
    type Error = anyhow::Error;

    /// Gets block at given height
    /// If block is not available, waits until it is
    /// It is possible to read non-finalized and last finalized blocks multiple times
    /// Finalized blocks must be read in order.
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        // Block until there's something
        self.wait_for_height(height).await?;
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

        // Block that precedes last finalized block is evicted at first read.
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
        let (blocks_len, oldest_available_height) = {
            let blocks = self.blocks.read().await;
            let blocks_len = blocks.len();
            let oldest_available_height = blocks.get(0).map(|b| b.header().height()).unwrap_or(0);
            (blocks_len, oldest_available_height)
        };
        let last_finalized_height = self.last_finalized_height.load(Ordering::Acquire);
        if blocks_len < self.blocks_to_finality as usize + 1 {
            let earliest_finalized_height = oldest_available_height
                .checked_add(self.blocks_to_finality as u64)
                .unwrap_or(0);
            self.wait_for_height(earliest_finalized_height).await?;
        }

        let blocks = self.blocks.read().await;
        let oldest_available_height = blocks[0].header().height();
        let index = last_finalized_height
            .checked_sub(oldest_available_height)
            .expect("Inconsistent MockDa");

        Ok(*blocks[index as usize].header())
    }

    async fn subscribe_finalized_header(&self) -> Result<Self::HeaderStream, Self::Error> {
        let receiver = self.finalized_header_sender.subscribe();
        Ok(MockDaBlockHeaderStream::new(receiver))
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

            if last_finalized_index > 0 {
                assert_eq!(next_index_to_finalize as u64, last_finalized_index + 1);
            }

            self.finalized_header_sender
                .send(*blocks[next_index_to_finalize].header())
                .unwrap();

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
    use tokio_stream::StreamExt;

    use super::*;

    #[tokio::test]
    async fn test_empty() {
        let mut da = MockDaService::new(MockAddress::new([1; 32]));
        da.wait_attempts = 10;

        let last_finalized_block_response = da.get_last_finalized_block_header().await;
        assert!(last_finalized_block_response.is_err());
        assert_eq!(
            "No blob at height=0 has been sent in time",
            last_finalized_block_response.err().unwrap().to_string()
        );
        let head_block_header_response = da.get_head_block_header().await;
        assert!(head_block_header_response.is_err());
        assert_eq!(
            "MockChain is empty",
            head_block_header_response.err().unwrap().to_string()
        );
    }

    async fn get_finalized_headers_collector(
        da: &mut MockDaService,
        expected_num_headers: usize,
    ) -> JoinHandle<Vec<MockBlockHeader>> {
        let mut receiver: MockDaBlockHeaderStream = da.subscribe_finalized_header().await.unwrap();
        // All finalized headers should be pushed by that time
        // This prevents test for freezing in case of a bug
        // But we need to wait longer, as `MockDa
        let timeout_duration = Duration::from_millis(1000);
        tokio::spawn(async move {
            let mut received = Vec::with_capacity(expected_num_headers);
            for _ in 0..expected_num_headers {
                match time::timeout(timeout_duration, receiver.next()).await {
                    Ok(Some(Ok(header))) => received.push(header),
                    _ => break,
                }
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
            assert!(response
                .err()
                .unwrap()
                .to_string()
                .starts_with("No blob at height="));
        }
    }

    async fn test_push_and_read(finalization: u64, num_blocks: usize) {
        let mut da = MockDaService::with_finality(MockAddress::new([1; 32]), finalization as u32);
        da.wait_attempts = 2;
        let number_of_finalized_blocks = num_blocks - finalization as usize;
        let collector_handle =
            get_finalized_headers_collector(&mut da, number_of_finalized_blocks).await;

        for i in 0..num_blocks {
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
        let expected_heights: Vec<u64> = (0..number_of_finalized_blocks as u64).collect();
        assert_eq!(expected_heights, heights);
    }

    async fn test_push_many_then_read(finalization: u64, num_blocks: usize) {
        let mut da = MockDaService::with_finality(MockAddress::new([1; 32]), finalization as u32);
        da.wait_attempts = 2;
        let number_of_finalized_blocks = num_blocks - finalization as usize;
        let collector_handle =
            get_finalized_headers_collector(&mut da, number_of_finalized_blocks).await;

        let blobs: Vec<Vec<u8>> = (0..num_blocks).map(|i| vec![i as u8; i + 1]).collect();

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

        // Starts from 0
        let expected_head_height = num_blocks as u64 - 1;
        let expected_finalized_height = expected_head_height - finalization;

        // Then read
        for (i, blob) in blobs.into_iter().enumerate() {
            let i = i as u64;

            let mut fetched_block = da.get_block_at(i).await.unwrap();
            assert_eq!(i, fetched_block.header().height());

            let last_finalized_header = da.get_last_finalized_block_header().await.unwrap();
            assert_eq!(expected_finalized_height, last_finalized_header.height());

            assert_eq!(&blob, fetched_block.blobs[0].full_data());

            let head_block_header = da.get_head_block_header().await.unwrap();
            assert_eq!(expected_head_height, head_block_header.height());
        }

        let received = collector_handle.await.unwrap();
        let finalized_heights: Vec<u64> = received.iter().map(|h| h.height()).collect();
        let expected_finalized_heights: Vec<u64> = (0..number_of_finalized_blocks as u64).collect();
        assert_eq!(expected_finalized_heights, finalized_heights);
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
