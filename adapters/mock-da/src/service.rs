use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use pin_project::pin_project;
use sha2::Digest;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec, Time};
use sov_rollup_interface::maybestd::sync::Arc;
use sov_rollup_interface::services::da::{DaService, SlotData};
use tokio::sync::{broadcast, RwLock, RwLockWriteGuard};
use tokio::time;

use crate::types::{MockAddress, MockBlob, MockBlock, MockDaVerifier};
use crate::verifier::MockDaSpec;
use crate::{MockBlockHeader, MockHash};

const GENESIS_HEADER: MockBlockHeader = MockBlockHeader {
    prev_hash: MockHash([0; 32]),
    hash: MockHash([1; 32]),
    height: 0,
    // 2023-01-01T00:00:00Z
    time: Time::from_secs(1672531200),
};

/// Definition of a fork that will be executed in `MockDaService` at specified height
pub struct PlannedFork {
    trigger_at_height: u64,
    fork_height: u64,
    blobs: Vec<Vec<u8>>,
}

impl PlannedFork {
    /// Creates new [`PlannedFork`]. Panics if some parameters are invalid.
    ///
    /// # Arguments
    ///
    /// * `trigger_at_height` - Height at which fork is "noticed".
    /// * `fork_height` - Height at which chain forked. Height of the first block in `blobs` will be `fork_height + 1`
    /// * `blobs` - Blobs that will be added after fork. Single blob per each block
    pub fn new(trigger_at_height: u64, fork_height: u64, blobs: Vec<Vec<u8>>) -> Self {
        if fork_height > trigger_at_height {
            panic!("Fork height must be less than trigger height");
        }
        let fork_len = (trigger_at_height - fork_height) as usize;
        if fork_len < blobs.len() {
            panic!("Not enough blobs for fork to be produced at given height");
        }
        Self {
            trigger_at_height,
            fork_height,
            blobs,
        }
    }
}

#[derive(Clone)]
/// DaService used in tests.
/// Currently only supports single blob per block.
/// Height of the first submitted block is 1.
/// Submitted blocks are kept indefinitely in memory.
pub struct MockDaService {
    sequencer_da_address: MockAddress,
    blocks: Arc<RwLock<VecDeque<MockBlock>>>,
    /// How many blocks should be submitted, before block is finalized. 0 means instant finality.
    blocks_to_finality: u32,
    /// Used for calculating correct finality from state of `blocks`
    finalized_header_sender: broadcast::Sender<MockBlockHeader>,
    wait_attempts: usize,
    planned_fork: Arc<Mutex<Option<PlannedFork>>>,
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
            finalized_header_sender: tx,
            wait_attempts: 100_0000,
            planned_fork: Arc::new(Mutex::new(None)),
        }
    }

    /// Get sequencer address
    pub fn get_sequencer_address(&self) -> MockAddress {
        self.sequencer_da_address
    }

    /// Change number of wait attempts before giving up on waiting for block
    pub fn set_wait_attempts(&mut self, wait_attempts: usize) {
        self.wait_attempts = wait_attempts;
    }

    async fn wait_for_height(&self, height: u64) -> anyhow::Result<()> {
        // Waits self.wait_attempts * 10ms to get block at height
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
        anyhow::bail!(
            "No block at height={height} has been sent in {:?}",
            Duration::from_millis((self.wait_attempts * 10) as u64),
        );
    }

    /// Rewrites existing non finalized blocks with given blocks
    /// New blobs will be added **after** specified height,
    /// meaning that first blob will be in the block of height + 1.
    pub async fn fork_at(&self, height: u64, blobs: Vec<Vec<u8>>) -> anyhow::Result<()> {
        let mut blocks = self.blocks.write().await;
        let last_finalized_height = self.get_last_finalized_height(&blocks).await;
        if last_finalized_height > height {
            anyhow::bail!(
                "Cannot fork at height {}, last finalized height is {}",
                height,
                last_finalized_height
            );
        }
        blocks.retain(|b| b.header().height <= height);
        for blob in blobs {
            let _ = self
                .add_blob(&blob, Default::default(), &mut blocks)
                .await?;
        }

        Ok(())
    }

    /// Set planned fork, that will be executed at specified height
    pub async fn set_planned_fork(&self, planned_fork: PlannedFork) -> anyhow::Result<()> {
        let last_finalized_height = {
            let blocks = self.blocks.write().await;
            self.get_last_finalized_height(&blocks).await
        };
        if last_finalized_height > planned_fork.trigger_at_height {
            anyhow::bail!(
                "Cannot fork at height {}, last finalized height is {}",
                planned_fork.trigger_at_height,
                last_finalized_height
            );
        }

        let mut fork = self.planned_fork.lock().unwrap();
        *fork = Some(planned_fork);

        Ok(())
    }

    async fn get_last_finalized_height(
        &self,
        blocks: &RwLockWriteGuard<'_, VecDeque<MockBlock>>,
    ) -> u64 {
        blocks
            .len()
            .checked_sub(self.blocks_to_finality as usize)
            .unwrap_or_default() as u64
    }

    async fn add_blob(
        &self,
        blob: &[u8],
        zkp_proof: Vec<u8>,
        blocks: &mut RwLockWriteGuard<'_, VecDeque<MockBlock>>,
    ) -> anyhow::Result<u64> {
        let (previous_block_hash, height) = match blocks.iter().last().map(|b| b.header().clone()) {
            None => (GENESIS_HEADER.hash(), GENESIS_HEADER.height() + 1),
            Some(block_header) => (block_header.hash(), block_header.height + 1),
        };

        let data_hash = hash_to_array(blob);
        let proof_hash = hash_to_array(&zkp_proof);
        // Hash only from single blob
        let block_hash = block_hash(height, data_hash, proof_hash, previous_block_hash.into());

        let blob = MockBlob::new_with_zkp_proof(
            blob.to_vec(),
            zkp_proof,
            self.sequencer_da_address,
            data_hash,
        );
        let header = MockBlockHeader {
            prev_hash: previous_block_hash,
            hash: block_hash,
            height,
            time: Time::now(),
        };
        let block = MockBlock {
            header,
            validity_cond: Default::default(),
            blobs: vec![blob],
        };
        blocks.push_back(block);

        // Enough blocks to finalize block
        if blocks.len() > self.blocks_to_finality as usize {
            let next_index_to_finalize = blocks.len() - self.blocks_to_finality as usize - 1;
            let next_finalized_header = blocks[next_index_to_finalize].header().clone();
            self.finalized_header_sender
                .send(next_finalized_header)
                .unwrap();
        }

        Ok(height)
    }

    /// Executes planned fork if it is planned at given height
    async fn planned_fork_handler(&self, height: u64) -> anyhow::Result<()> {
        let planned_fork_now = {
            let mut planned_fork_guard = self.planned_fork.lock().unwrap();
            if planned_fork_guard
                .as_ref()
                .map_or(false, |x| x.trigger_at_height == height)
            {
                Some(planned_fork_guard.take().unwrap())
            } else {
                None
            }
        };
        if let Some(planned_fork_now) = planned_fork_now {
            self.fork_at(planned_fork_now.fork_height, planned_fork_now.blobs)
                .await?;
        }
        Ok(())
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
    type TransactionId = ();
    type Error = anyhow::Error;

    /// Gets block at given height
    /// If block is not available, waits until it is
    /// It is possible to read non-finalized and last finalized blocks multiple times
    /// Finalized blocks must be read in order.
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        if height == 0 {
            anyhow::bail!("The lowest queryable block should be > 0");
        }
        // Fork logic
        self.planned_fork_handler(height).await?;
        // Block until there's something
        self.wait_for_height(height).await?;
        // Locking blocks here, so submissions has to wait
        let blocks = self.blocks.write().await;
        let oldest_available_height = blocks[0].header.height;
        let index = height
            .checked_sub(oldest_available_height)
            .ok_or(anyhow::anyhow!(
                "Block at height {} is not available anymore",
                height
            ))?;

        Ok(blocks.get(index as usize).unwrap().clone())
    }

    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let blocks_len = { self.blocks.read().await.len() };
        if blocks_len < self.blocks_to_finality as usize + 1 {
            return Ok(GENESIS_HEADER);
        }

        let blocks = self.blocks.read().await;
        let index = blocks_len - self.blocks_to_finality as usize - 1;
        Ok(blocks[index].header().clone())
    }

    async fn subscribe_finalized_header(&self) -> Result<Self::HeaderStream, Self::Error> {
        let receiver = self.finalized_header_sender.subscribe();
        Ok(MockDaBlockHeaderStream::new(receiver))
    }

    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let blocks = self.blocks.read().await;

        Ok(blocks
            .iter()
            .last()
            .map(|b| b.header().clone())
            .unwrap_or(GENESIS_HEADER))
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
        let _ = self.add_blob(blob, Default::default(), &mut blocks).await?;
        Ok(())
    }

    async fn send_aggregated_zk_proof(&self, proof: &[u8]) -> Result<u64, Self::Error> {
        let mut blocks = self.blocks.write().await;
        self.add_blob(Default::default(), proof.to_vec(), &mut blocks)
            .await
    }

    async fn get_aggregated_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        let blobs = self.get_block_at(height).await?.blobs;
        Ok(blobs.into_iter().map(|b| b.zk_proofs_data).collect())
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

fn block_hash(
    height: u64,
    data_hash: [u8; 32],
    proof_hash: [u8; 32],
    prev_hash: [u8; 32],
) -> MockHash {
    let mut block_to_hash = height.to_be_bytes().to_vec();
    block_to_hash.extend_from_slice(&data_hash[..]);
    block_to_hash.extend_from_slice(&proof_hash[..]);
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

        let last_finalized_header = da.get_last_finalized_block_header().await.unwrap();
        assert_eq!(GENESIS_HEADER, last_finalized_header);

        let head_header = da.get_head_block_header().await.unwrap();
        assert_eq!(GENESIS_HEADER, head_header);

        let zero_block = da.get_block_at(0).await;
        assert!(zero_block.is_err());
        assert_eq!(
            "The lowest queryable block should be > 0",
            zero_block.unwrap_err().to_string()
        );

        {
            let has_planned_fork = da.planned_fork.lock().unwrap();
            assert!(has_planned_fork.is_none());
        }
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
        let finalized_header = response.unwrap();
        if let Some(expected_finalized_height) = submit_height.checked_sub(blocks_to_finalization) {
            assert_eq!(expected_finalized_height, finalized_header.height());
        } else {
            assert_eq!(GENESIS_HEADER, finalized_header);
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
            let height = (i + 1) as u64;

            da.send_transaction(&published_blob).await.unwrap();

            let mut block = da.get_block_at(height).await.unwrap();

            assert_eq!(height, block.header.height());
            assert_eq!(1, block.blobs.len());
            let blob = &mut block.blobs[0];
            let retrieved_data = blob.full_data().to_vec();
            assert_eq!(published_blob, retrieved_data);

            let last_finalized_block_response = da.get_last_finalized_block_header().await;
            validate_get_finalized_header_response(
                height,
                finalization,
                last_finalized_block_response,
            );
        }

        let received = collector_handle.await.unwrap();
        let heights: Vec<u64> = received.iter().map(|h| h.height()).collect();
        let expected_heights: Vec<u64> = (1..=number_of_finalized_blocks as u64).collect();
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
            let height = (i + 1) as u64;
            // Send transaction should pass
            da.send_transaction(blob).await.unwrap();
            let last_finalized_block_response = da.get_last_finalized_block_header().await;
            validate_get_finalized_header_response(
                height,
                finalization,
                last_finalized_block_response,
            );

            let head_block_header = da.get_head_block_header().await.unwrap();
            assert_eq!(height, head_block_header.height());
        }

        // Starts from 0
        let expected_head_height = num_blocks as u64;
        let expected_finalized_height = expected_head_height - finalization;

        // Then read
        for (i, blob) in blobs.into_iter().enumerate() {
            let i = (i + 1) as u64;

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
        let expected_finalized_heights: Vec<u64> =
            (1..=number_of_finalized_blocks as u64).collect();
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

        #[tokio::test]
        async fn read_multiple_times() {
            let mut da = MockDaService::with_finality(MockAddress::new([1; 32]), 4);
            da.wait_attempts = 2;

            // 1 -> 2 -> 3

            da.send_transaction(&[1, 2, 3, 4]).await.unwrap();
            da.send_transaction(&[4, 5, 6, 7]).await.unwrap();
            da.send_transaction(&[8, 9, 0, 1]).await.unwrap();

            let block_1_before = da.get_block_at(1).await.unwrap();
            let block_2_before = da.get_block_at(2).await.unwrap();
            let block_3_before = da.get_block_at(3).await.unwrap();

            let result = da.get_block_at(4).await;
            assert!(result.is_err());

            let block_1_after = da.get_block_at(1).await.unwrap();
            let block_2_after = da.get_block_at(2).await.unwrap();
            let block_3_after = da.get_block_at(3).await.unwrap();

            assert_eq!(block_1_before, block_1_after);
            assert_eq!(block_2_before, block_2_after);
            assert_eq!(block_3_before, block_3_after);
            // Just some sanity check
            assert_ne!(block_1_before, block_2_before);
            assert_ne!(block_3_before, block_1_before);
            assert_ne!(block_1_before, block_2_after);
        }
    }

    #[tokio::test]
    async fn test_zk_submission() -> Result<(), anyhow::Error> {
        let da = MockDaService::new(MockAddress::new([1; 32]));
        let aggregated_proof_data = vec![1, 2, 3];
        let height = da.send_aggregated_zk_proof(&aggregated_proof_data).await?;
        let proofs = da.get_aggregated_proofs_at(height).await?;

        assert_eq!(vec![aggregated_proof_data], proofs);
        Ok(())
    }

    mod reo4g_control {
        use super::*;
        use crate::{MockAddress, MockDaService};

        #[tokio::test]
        async fn test_reorg_control_success() {
            let da = MockDaService::with_finality(MockAddress::new([1; 32]), 4);

            // 1 -> 2 -> 3.1 -> 4.1
            //      \ -> 3.2 -> 4.2

            // 1
            da.send_transaction(&[1, 2, 3, 4]).await.unwrap();
            // 2
            da.send_transaction(&[4, 5, 6, 7]).await.unwrap();
            // 3.1
            da.send_transaction(&[8, 9, 0, 1]).await.unwrap();
            // 4.1
            da.send_transaction(&[2, 3, 4, 5]).await.unwrap();

            let _block_1 = da.get_block_at(1).await.unwrap();
            let block_2 = da.get_block_at(2).await.unwrap();
            let block_3 = da.get_block_at(3).await.unwrap();
            let head_before = da.get_head_block_header().await.unwrap();

            // Do reorg
            da.fork_at(2, vec![vec![3, 3, 3, 3], vec![4, 4, 4, 4]])
                .await
                .unwrap();

            let block_3_after = da.get_block_at(3).await.unwrap();
            assert_ne!(block_3, block_3_after);

            assert_eq!(block_2.header().hash(), block_3_after.header().prev_hash());

            let head_after = da.get_head_block_header().await.unwrap();
            assert_ne!(head_before, head_after);
        }

        #[tokio::test]
        async fn test_attempt_reorg_after_finalized() {
            let da = MockDaService::with_finality(MockAddress::new([1; 32]), 2);

            // 1 -> 2 -> 3 -> 4

            da.send_transaction(&[1, 2, 3, 4]).await.unwrap();
            da.send_transaction(&[4, 5, 6, 7]).await.unwrap();
            da.send_transaction(&[8, 9, 0, 1]).await.unwrap();
            da.send_transaction(&[2, 3, 4, 5]).await.unwrap();

            let block_1_before = da.get_block_at(1).await.unwrap();
            let block_2_before = da.get_block_at(2).await.unwrap();
            let block_3_before = da.get_block_at(3).await.unwrap();
            let block_4_before = da.get_block_at(4).await.unwrap();
            let finalized_header_before = da.get_last_finalized_block_header().await.unwrap();
            assert_eq!(&finalized_header_before, block_2_before.header());

            // Attempt at finalized header. It will try to overwrite height 2 and 3
            let result = da
                .fork_at(1, vec![vec![3, 3, 3, 3], vec![4, 4, 4, 4]])
                .await;
            assert!(result.is_err());
            assert_eq!(
                "Cannot fork at height 1, last finalized height is 2",
                result.unwrap_err().to_string()
            );

            let block_1_after = da.get_block_at(1).await.unwrap();
            let block_2_after = da.get_block_at(2).await.unwrap();
            let block_3_after = da.get_block_at(3).await.unwrap();
            let block_4_after = da.get_block_at(4).await.unwrap();
            let finalized_header_after = da.get_last_finalized_block_header().await.unwrap();
            assert_eq!(&finalized_header_after, block_2_after.header());

            assert_eq!(block_1_before, block_1_after);
            assert_eq!(block_2_before, block_2_after);
            assert_eq!(block_3_before, block_3_after);
            assert_eq!(block_4_before, block_4_after);

            // Overwriting height 3 and 4 is ok
            let result2 = da
                .fork_at(2, vec![vec![3, 3, 3, 3], vec![4, 4, 4, 4]])
                .await;
            assert!(result2.is_ok());
            let block_2_after_reorg = da.get_block_at(2).await.unwrap();
            let block_3_after_reorg = da.get_block_at(3).await.unwrap();

            assert_eq!(block_2_after, block_2_after_reorg);
            assert_ne!(block_3_after, block_3_after_reorg);
        }

        #[tokio::test]
        async fn test_planned_reorg() {
            let mut da = MockDaService::with_finality(MockAddress::new([1; 32]), 4);
            da.wait_attempts = 2;

            // Planned for will replace blocks at height 3 and 4
            let planned_fork = PlannedFork::new(4, 2, vec![vec![3, 3, 3, 3], vec![4, 4, 4, 4]]);

            da.set_planned_fork(planned_fork).await.unwrap();
            {
                let has_planned_fork = da.planned_fork.lock().unwrap();
                assert!(has_planned_fork.is_some());
            }

            da.send_transaction(&[1, 2, 3, 4]).await.unwrap();
            da.send_transaction(&[4, 5, 6, 7]).await.unwrap();
            da.send_transaction(&[8, 9, 0, 1]).await.unwrap();

            let block_1_before = da.get_block_at(1).await.unwrap();
            let block_2_before = da.get_block_at(2).await.unwrap();
            assert_consecutive_blocks(&block_1_before, &block_2_before);
            let block_3_before = da.get_block_at(3).await.unwrap();
            assert_consecutive_blocks(&block_2_before, &block_3_before);
            let block_4 = da.get_block_at(4).await.unwrap();
            {
                let has_planned_fork = da.planned_fork.lock().unwrap();
                assert!(!has_planned_fork.is_some());
            }

            // Fork is happening!
            assert_ne!(block_3_before.header().hash(), block_4.header().prev_hash());
            let block_3_after = da.get_block_at(3).await.unwrap();
            assert_consecutive_blocks(&block_3_after, &block_4);
            assert_consecutive_blocks(&block_2_before, &block_3_after);
        }

        #[tokio::test]
        async fn test_planned_reorg_shorter() {
            let mut da = MockDaService::with_finality(MockAddress::new([1; 32]), 4);
            da.wait_attempts = 2;
            // Planned for will replace blocks at height 3 and 4
            let planned_fork =
                PlannedFork::new(4, 2, vec![vec![13, 13, 13, 13], vec![14, 14, 14, 14]]);
            da.set_planned_fork(planned_fork).await.unwrap();

            da.send_transaction(&[1, 1, 1, 1]).await.unwrap();
            da.send_transaction(&[2, 2, 2, 2]).await.unwrap();
            da.send_transaction(&[3, 3, 3, 3]).await.unwrap();
            da.send_transaction(&[4, 4, 4, 4]).await.unwrap();
            da.send_transaction(&[5, 5, 5, 5]).await.unwrap();

            let block_1_before = da.get_block_at(1).await.unwrap();
            let block_2_before = da.get_block_at(2).await.unwrap();
            assert_consecutive_blocks(&block_1_before, &block_2_before);
            let block_3_before = da.get_block_at(3).await.unwrap();
            assert_consecutive_blocks(&block_2_before, &block_3_before);
            let block_4 = da.get_block_at(4).await.unwrap();
            assert_ne!(block_4.header().prev_hash(), block_3_before.header().hash());
            let block_1_after = da.get_block_at(1).await.unwrap();
            let block_2_after = da.get_block_at(2).await.unwrap();
            let block_3_after = da.get_block_at(3).await.unwrap();
            assert_consecutive_blocks(&block_3_after, &block_4);
            assert_consecutive_blocks(&block_2_after, &block_3_after);
            assert_consecutive_blocks(&block_1_after, &block_2_after);

            let block_5 = da.get_block_at(5).await;
            assert_eq!(
                "No block at height=5 has been sent in 20ms",
                block_5.unwrap_err().to_string()
            );
        }
    }

    fn assert_consecutive_blocks(block1: &MockBlock, block2: &MockBlock) {
        assert_eq!(block2.header().prev_hash(), block1.header().hash())
    }
}
