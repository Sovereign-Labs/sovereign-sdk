use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{
    BlobReaderTrait, BlockHeaderTrait, DaSpec, RelevantBlobs, RelevantProofs, Time,
};
use sov_rollup_interface::node::da::{DaService, MaybeRetryable, SlotData, SubmitBlobReceipt};
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};
use tokio::time;

use crate::config::{GENESIS_BLOCK, GENESIS_HEADER, WAIT_ATTEMPT_PAUSE};
use crate::in_memory::fork::PlannedFork;
use crate::utils::hash_to_array;
use crate::{
    MockAddress, MockBlob, MockBlock, MockBlockHeader, MockDaConfig, MockDaSpec, MockDaVerifier,
    MockHash,
};

const DEFAULT_WAIT_ATTEMPTS: u64 = 100;

/// A [`DaService`] for use in tests.
///
/// The height of the first submitted block is 1.
/// Submitted blocks are kept indefinitely in memory.
#[derive(Clone, Debug)]
pub struct MockDaService {
    sequencer_da_address: MockAddress,
    aggregated_proof_buffer: Arc<Mutex<VecDeque<MockBlob>>>,
    blocks: Arc<RwLock<VecDeque<MockBlock>>>,
    /// Defines how many blocks should be submitted, before block becomes finalized.
    /// Zero means instant finality.
    blocks_to_finality: u32,
    /// Used for calculating correct finality from state of `blocks`.
    finalized_header_sender: broadcast::Sender<MockBlockHeader>,
    /// How many attempts to get block at given height this service is going to do before giving up.
    /// Wait time between attempts is defined by [`WAIT_ATTEMPT_PAUSE`].
    wait_attempts: u64,
    planned_fork: Option<PlannedFork>,
    aggregated_proof_sender: broadcast::Sender<()>,
}

/// Allows consuming the [`futures::Stream`] of BlockHeaders.
type HeaderStream = BoxStream<'static, Result<MockBlockHeader, anyhow::Error>>;

impl MockDaService {
    /// Creates a new [`MockDaService`] with instant finality.
    pub fn new(sequencer_da_address: MockAddress) -> Self {
        let (tx, mut rx) = broadcast::channel(100);

        // Spawn a task, so the receiver is not dropped and the channel is not
        // closed. Once the sender is dropped, the receiver will receive an
        // error and the task will exit.
        tokio::spawn(async move { while rx.recv().await.is_ok() {} });

        let (aggregated_proof_subscription, mut rec) = broadcast::channel(16);
        tokio::spawn(async move { while rec.recv().await.is_ok() {} });
        let mut blocks: VecDeque<MockBlock> = Default::default();
        blocks.push_back(GENESIS_BLOCK.clone());
        let blocks = Arc::new(RwLock::new(blocks));
        Self {
            sequencer_da_address,
            aggregated_proof_buffer: Default::default(),
            blocks,
            blocks_to_finality: 0,
            finalized_header_sender: tx,
            wait_attempts: DEFAULT_WAIT_ATTEMPTS,
            planned_fork: None,
            aggregated_proof_sender: aggregated_proof_subscription,
        }
    }

    /// Sets the desired distance between the last finalized block and the head
    /// block.
    pub fn with_finality(mut self, blocks_to_finality: u32) -> Self {
        self.blocks_to_finality = blocks_to_finality;
        self
    }

    /// Sets the number of wait attempts before giving up on waiting for a block.
    pub fn with_wait_attempts(mut self, wait_attempts: u64) -> Self {
        self.wait_attempts = wait_attempts;
        self
    }

    /// Returns the sequencer's address.
    pub fn sequencer_address(&self) -> MockAddress {
        self.sequencer_da_address
    }

    async fn wait_for_height(&self, height: u64) -> anyhow::Result<()> {
        let start = Instant::now();
        // Waits self.wait_attempts * [`WAIT_ATTEMPT_PAUSE`] to get block at height
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
            time::sleep(WAIT_ATTEMPT_PAUSE).await;
        }
        anyhow::bail!(
            "No block at height={height} has been sent in {:?}",
            start.elapsed()
        );
    }

    /// Rewrites existing non finalized blocks with given blocks
    /// New blobs will be added **after** specified height,
    /// meaning that the first blob will be in the block of height + 1.
    pub async fn fork_at(&self, height: u64, tx_blobs: &[Vec<u8>]) -> anyhow::Result<()> {
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
        for blob in tx_blobs {
            let batch_blob = self.make_blob(blob.to_vec());
            let proof_blob = self.make_blob(Default::default());
            let _ = self.add_block(batch_blob, vec![proof_blob], &mut blocks);
        }

        Ok(())
    }

    /// Set planned fork, that will be executed at the specified height.
    pub async fn set_planned_fork(&mut self, planned_fork: PlannedFork) -> anyhow::Result<()> {
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

        self.planned_fork = Some(planned_fork);
        Ok(())
    }

    async fn get_last_finalized_height(&self, blocks: &VecDeque<MockBlock>) -> u64 {
        blocks
            .len()
            .checked_sub(self.blocks_to_finality as usize)
            .unwrap_or_default() as u64
    }

    fn make_blob(&self, blob: Vec<u8>) -> MockBlob {
        MockBlob::new_with_hash(blob, self.sequencer_da_address)
    }

    fn make_new_block(
        &self,
        batch_blob: MockBlob,
        proof_blobs: Vec<MockBlob>,
        blocks: &mut VecDeque<MockBlock>,
    ) -> MockBlock {
        let prev = blocks
            .iter()
            .last()
            .map(|b| b.header().clone())
            .unwrap_or(GENESIS_HEADER);

        let height = prev.height() + 1;

        let mut blob_hashes: Vec<_> = proof_blobs.iter().map(|b| b.hash).collect();
        blob_hashes.push(batch_blob.hash);

        let block_hash = block_hash(height, &blob_hashes, prev.hash().into());

        let header = MockBlockHeader {
            prev_hash: prev.hash(),
            hash: block_hash,
            height,
            time: Time::now(),
        };

        MockBlock {
            header,
            batch_blobs: vec![batch_blob],
            proof_blobs,
        }
    }

    /// In the [`MockDaService`] a single block contains only one batch blob and any number of proof blobs.
    fn add_block(
        &self,
        batch_blob: MockBlob,
        proof_blob: Vec<MockBlob>,
        blocks: &mut VecDeque<MockBlock>,
    ) -> (u64, MockHash) {
        let block = self.make_new_block(batch_blob, proof_blob, blocks);
        let hash = block.header.hash();

        let height = block.header.height;
        tracing::debug!("Creating block at height {}", height);
        blocks.push_back(block);

        // Enough blocks to finalize block
        if blocks.len() > self.blocks_to_finality as usize {
            let next_index_to_finalize = blocks.len() - self.blocks_to_finality as usize - 1;
            let next_finalized_header = blocks[next_index_to_finalize].header().clone();
            tracing::debug!("Finalizing block at height {}", next_index_to_finalize);
            self.finalized_header_sender
                .send(next_finalized_header)
                .unwrap();
        }

        (height, hash)
    }

    /// Executes planned fork if it is planned at a given height.
    async fn planned_fork_handler(&self, height: u64) -> anyhow::Result<()> {
        if let Some(planned_fork_now) = &self.planned_fork {
            if planned_fork_now.trigger_at_height == height {
                self.fork_at(
                    planned_fork_now.fork_height,
                    planned_fork_now.blobs.as_slice(),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Will receive notification one block before the proof is included on the DA.
    pub fn subscribe_proof_posted(&self) -> broadcast::Receiver<()> {
        self.aggregated_proof_sender.subscribe()
    }

    /// Subscribe to finalized headers as they are finalized.
    /// Expect only to receive headers which were finalized after subscription
    /// Optimized version of `get_last_finalized_block_header`.
    pub async fn subscribe_finalized_header(&self) -> Result<HeaderStream, anyhow::Error> {
        let receiver = self.finalized_header_sender.subscribe();
        let stream = futures::stream::unfold(receiver, |mut receiver| async move {
            match receiver.recv().await {
                Ok(header) => Some((Ok(header), receiver)),
                Err(_) => None,
            }
        });

        Ok(stream.boxed())
    }
}

fn block_hash(height: u64, blob_hashes: &[MockHash], prev_hash: [u8; 32]) -> MockHash {
    let mut block_to_hash = height.to_be_bytes().to_vec();

    for blob_hash in blob_hashes {
        block_to_hash.extend_from_slice(blob_hash.as_ref());
    }

    block_to_hash.extend_from_slice(&prev_hash);

    MockHash::from(hash_to_array(&block_to_hash))
}

#[async_trait]
impl DaService for MockDaService {
    type Spec = MockDaSpec;
    type Config = MockDaConfig;
    type Verifier = MockDaVerifier;
    type FilteredBlock = MockBlock;
    type Error = anyhow::Error;

    const GUARANTEES_TRANSACTION_ORDERING: bool = true;

    /// Gets block at given height
    /// If block is not available, waits until it is produced.
    /// It is possible to read non-finalized and last finalized blocks multiple times
    /// Finalized blocks must be read in order.
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        if height == 0 {
            return Ok(GENESIS_BLOCK);
        }

        // Fork logic
        self.planned_fork_handler(height)
            .await
            .map_err(MaybeRetryable::Transient)?;
        // Block until there's something
        self.wait_for_height(height)
            .await
            .map_err(MaybeRetryable::Transient)?;
        // Locking blocks here, so submissions have to wait
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

        let mut proof_buffer = self.aggregated_proof_buffer.lock().await;
        let mut proof_blobs = Vec::new();
        while let Some(blob) = proof_buffer.pop_front() {
            tracing::debug!("Including buffered proof in block");
            proof_blobs.push(blob);
        }
        let proofs_included = proof_blobs.len();
        let blob_size_bytes = blob.len();

        let mut blocks = self.blocks.write().await;
        let batch_blob = self.make_blob(blob.to_vec());

        let (height, blob_hash) = self.add_block(batch_blob, proof_blobs, &mut blocks);

        tracing::debug!(
            height,
            blob_size_bytes,
            proofs_included,
            "MockBlock has been saved"
        );

        let res = Ok(SubmitBlobReceipt {
            blob_hash: HexHash::new(blob_hash.0),
            da_transaction_id: blob_hash,
        });

        tx.send(res).unwrap();
        rx
    }

    /// Sends proof to the MockDA. The submitted proof is internally buffered and will be included on the MockDA
    /// alongside the next batch of transactions (after calling the `send_transaction` function).
    async fn send_proof(
        &self,
        proof: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        let (tx, rx) = oneshot::channel();

        tracing::debug!("Proof received. Buffering for later inclusion.");

        let proof_blob = self.make_blob(proof.to_vec());
        let blob_hash = proof_blob.hash();

        let mut proof_buffer = self.aggregated_proof_buffer.lock().await;
        proof_buffer.push_back(proof_blob);
        self.aggregated_proof_sender.send(()).unwrap();

        let res = Ok(SubmitBlobReceipt {
            blob_hash: HexHash::new(blob_hash.0),
            da_transaction_id: blob_hash,
        });

        tx.send(res).unwrap();
        rx
    }

    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        let blobs = self.get_block_at(height).await?.proof_blobs;
        Ok(blobs
            .into_iter()
            .map(|mut proof_blob| proof_blob.full_data().to_vec())
            .collect())
    }

    async fn get_signer(&self) -> <Self::Spec as DaSpec>::Address {
        self.sequencer_da_address
    }
}

#[cfg(test)]
mod tests {
    use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait};
    use tokio::task::JoinHandle;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_empty() {
        let mut da = MockDaService::new(MockAddress::new([1; 32]));
        da.wait_attempts = 10;

        let last_finalized_header = da.get_last_finalized_block_header().await.unwrap();
        assert_eq!(GENESIS_HEADER, last_finalized_header);

        let head_header = da.get_head_block_header().await.unwrap();
        assert_eq!(GENESIS_HEADER, head_header);

        let zero_block = da.get_block_at(0).await;
        assert_eq!(zero_block.unwrap().header(), &GENESIS_HEADER);
    }

    async fn get_finalized_headers_collector(
        da: &mut MockDaService,
        expected_num_headers: usize,
    ) -> JoinHandle<Vec<MockBlockHeader>> {
        let mut receiver = da.subscribe_finalized_header().await.unwrap();
        // All finalized headers should be pushed by that time
        // This prevents test for freezing in case of a bug,
        // But we need to wait longer, as `MockDa
        let timeout_duration = time::Duration::from_millis(1000);
        tokio::spawn(async move {
            let mut received = Vec::with_capacity(expected_num_headers);
            for _ in 0..=expected_num_headers {
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

    async fn test_push_and_read(finalization: u64, num_blocks: usize) -> anyhow::Result<()> {
        let mut da = MockDaService::new(MockAddress::new([1; 32])).with_finality(finalization as _);
        da.wait_attempts = 2;
        let number_of_finalized_blocks = num_blocks - finalization as usize;
        let collector_handle =
            get_finalized_headers_collector(&mut da, number_of_finalized_blocks).await;

        for i in 1..num_blocks {
            let published_blob: Vec<u8> = vec![i as u8; i + 1];
            let height = i as u64;

            da.send_transaction(&published_blob).await.await??;
            let mut block = da.get_block_at(height).await?;

            assert_eq!(height, block.header.height());
            assert_eq!(1, block.batch_blobs.len());
            let blob = &mut block.batch_blobs[0];
            let retrieved_data = blob.full_data().to_vec();
            assert_eq!(published_blob, retrieved_data);

            let last_finalized_block_response = da.get_last_finalized_block_header().await?;
            validate_get_finalized_header_response(
                height,
                finalization,
                Ok(last_finalized_block_response),
            );
        }

        let received = collector_handle.await?;
        let heights: Vec<u64> = received.iter().map(|h| h.height()).collect();
        // When finalization is set to zero, the DA service sends the notification for the Genesis block
        // before we subscribe, so we miss that one.
        let start_height = if finalization == 0 { 1 } else { 0 };
        let expected_heights: Vec<u64> =
            (start_height..number_of_finalized_blocks as u64).collect();
        assert_eq!(expected_heights, heights);

        Ok(())
    }

    async fn test_push_many_then_read(finalization: u64, num_blocks: usize) -> anyhow::Result<()> {
        let mut da = MockDaService::new(MockAddress::new([1; 32])).with_finality(finalization as _);
        da.wait_attempts = 2;
        let number_of_finalized_blocks = num_blocks - finalization as usize;
        let collector_handle =
            get_finalized_headers_collector(&mut da, number_of_finalized_blocks).await;

        let blobs: Vec<Vec<u8>> = (0..num_blocks).map(|i| vec![i as u8; i + 1]).collect();

        // Submitting blobs first
        for (i, blob) in blobs.iter().enumerate() {
            let height = (i + 1) as u64;
            // Send transaction should pass
            da.send_transaction(blob).await.await??;
            let last_finalized_block_response = da.get_last_finalized_block_header().await.unwrap();
            validate_get_finalized_header_response(
                height,
                finalization,
                Ok(last_finalized_block_response),
            );

            let head_block_header = da.get_head_block_header().await?;
            assert_eq!(height, head_block_header.height());
        }

        // Starts from 0
        let expected_head_height = num_blocks as u64;
        let expected_finalized_height = expected_head_height - finalization;

        // Then read
        for (i, blob) in blobs.into_iter().enumerate() {
            let i = (i + 1) as u64;

            let mut fetched_block = da.get_block_at(i).await?;
            assert_eq!(i, fetched_block.header().height());

            let last_finalized_header = da.get_last_finalized_block_header().await?;
            assert_eq!(expected_finalized_height, last_finalized_header.height());

            assert_eq!(&blob, fetched_block.batch_blobs[0].full_data());

            let head_block_header = da.get_head_block_header().await?;
            assert_eq!(expected_head_height, head_block_header.height());
        }

        let received = collector_handle.await?;
        let finalized_heights: Vec<u64> = received.iter().map(|h| h.height()).collect();
        // When finalization is set to zero, the DA service sends the notification for the Genesis block
        // before we subscribe, so we miss that one.
        let start_height = if finalization == 0 { 1 } else { 0 };
        let expected_finalized_heights: Vec<u64> =
            (start_height..=number_of_finalized_blocks as u64).collect();
        assert_eq!(expected_finalized_heights, finalized_heights);

        Ok(())
    }

    mod instant_finality {
        use super::*;

        // FIXME(@neysofu): Multi-threaded Tokio runtime but the test name is
        // `...single_thread`.
        //
        // I haven't looked into this and checked whether it's actually a problem,
        // but it sure sounds like it.
        #[tokio::test(flavor = "multi_thread")]
        /// Pushing a blob and immediately reading it
        async fn flaky_push_pull_single_thread() {
            test_push_and_read(0, 10).await.unwrap();
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn push_many_then_read() {
            test_push_many_then_read(0, 10).await.unwrap();
        }
    }

    mod non_instant_finality {
        use super::*;

        // FIXME(@neysofu): Multi-threaded Tokio runtime but the test name is
        // `...single_thread`.
        //
        // I haven't looked into this and checked whether it's actually a problem,
        // but it sure sounds like it.
        #[tokio::test(flavor = "multi_thread")]
        async fn flaky_push_pull_single_thread() {
            test_push_and_read(1, 10).await.unwrap();
            test_push_and_read(3, 10).await.unwrap();
            test_push_and_read(5, 10).await.unwrap();
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn push_many_then_read() {
            test_push_many_then_read(1, 10).await.unwrap();
            test_push_many_then_read(3, 10).await.unwrap();
            test_push_many_then_read(5, 10).await.unwrap();
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn read_multiple_times() -> anyhow::Result<()> {
            let mut da = MockDaService::new(MockAddress::new([1; 32])).with_finality(4);
            da.wait_attempts = 2;

            // 1 -> 2 -> 3
            da.send_transaction(&[1, 2, 3, 4]).await.await??;
            da.send_transaction(&[4, 5, 6, 7]).await.await??;
            da.send_transaction(&[8, 9, 0, 1]).await.await??;

            let block_1_before = da.get_block_at(1).await?;
            let block_2_before = da.get_block_at(2).await?;
            let block_3_before = da.get_block_at(3).await?;

            let result = da.get_block_at(4).await;
            assert!(result.is_err());

            let block_1_after = da.get_block_at(1).await?;
            let block_2_after = da.get_block_at(2).await?;
            let block_3_after = da.get_block_at(3).await?;

            assert_eq!(block_1_before, block_1_after);
            assert_eq!(block_2_before, block_2_after);
            assert_eq!(block_3_before, block_3_after);
            // Just some sanity check
            assert_ne!(block_1_before, block_2_before);
            assert_ne!(block_3_before, block_1_before);
            assert_ne!(block_1_before, block_2_after);
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_zk_submission() -> anyhow::Result<()> {
        let da = MockDaService::new(MockAddress::new([1; 32]));
        let aggregated_proof_data = vec![1, 2, 3];
        da.send_proof(&aggregated_proof_data).await.await??;

        let tx_data = vec![1];
        da.send_transaction(&tx_data).await.await??;

        let proofs = da.get_proofs_at(1).await?;
        assert_eq!(vec![aggregated_proof_data], proofs);

        for i in 2..5 {
            let aggregated_proof_data = vec![i];
            da.send_proof(&aggregated_proof_data).await.await??;
        }
        let tx_data = vec![1];
        da.send_transaction(&tx_data).await.await??;

        let proofs = da.get_proofs_at(2).await?;
        assert_eq!(vec![vec![2], vec![3], vec![4]], proofs);

        Ok(())
    }

    mod reo4g_control {
        use super::*;

        #[tokio::test(flavor = "multi_thread")]
        async fn test_reorg_control_success() -> anyhow::Result<()> {
            let da = MockDaService::new(MockAddress::new([1; 32])).with_finality(4);

            // 1 -> 2 -> 3.1 -> 4.1
            //      \ -> 3.2 -> 4.2

            // 1
            da.send_transaction(&[1, 2, 3, 4]).await.await??;
            // 2
            da.send_transaction(&[4, 5, 6, 7]).await.await??;
            // 3.1
            da.send_transaction(&[8, 9, 0, 1]).await.await??;
            // 4.1
            da.send_transaction(&[2, 3, 4, 5]).await.await??;

            let _block_1 = da.get_block_at(1).await?;
            let block_2 = da.get_block_at(2).await?;
            let block_3 = da.get_block_at(3).await?;
            let head_before = da.get_head_block_header().await?;

            // Do reorg
            da.fork_at(2, &[vec![3, 3, 3, 3], vec![4, 4, 4, 4]]).await?;

            let block_3_after = da.get_block_at(3).await?;
            assert_ne!(block_3, block_3_after);

            assert_eq!(block_2.header().hash(), block_3_after.header().prev_hash());

            let head_after = da.get_head_block_header().await?;
            assert_ne!(head_before, head_after);

            Ok(())
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test_attempt_reorg_after_finalized() -> anyhow::Result<()> {
            let da = MockDaService::new(MockAddress::new([1; 32])).with_finality(3);

            // 1 -> 2 -> 3 -> 4

            da.send_transaction(&[1, 2, 3, 4]).await.await??;
            da.send_transaction(&[4, 5, 6, 7]).await.await??;

            da.send_transaction(&[8, 9, 0, 1]).await.await??;
            da.send_transaction(&[2, 3, 4, 5]).await.await??;

            let block_1_before = da.get_block_at(1).await?;
            let block_2_before = da.get_block_at(2).await?;
            let block_3_before = da.get_block_at(3).await?;
            let block_4_before = da.get_block_at(4).await?;
            let finalized_header_before = da.get_last_finalized_block_header().await?;
            assert_eq!(&finalized_header_before, block_1_before.header());

            // Attempt at finalized header. It will try to overwrite height 2 and 3
            let result = da.fork_at(1, &[vec![3, 3, 3, 3], vec![4, 4, 4, 4]]).await;
            assert!(result.is_err());
            assert_eq!(
                "Cannot fork at height 1, last finalized height is 2",
                result.unwrap_err().to_string()
            );

            let block_1_after = da.get_block_at(1).await?;
            let block_2_after = da.get_block_at(2).await?;
            let block_3_after = da.get_block_at(3).await?;
            let block_4_after = da.get_block_at(4).await?;
            let finalized_header_after = da.get_last_finalized_block_header().await?;
            assert_eq!(&finalized_header_after, block_1_after.header());

            assert_eq!(block_1_before, block_1_after);
            assert_eq!(block_2_before, block_2_after);
            assert_eq!(block_3_before, block_3_after);
            assert_eq!(block_4_before, block_4_after);

            // Overwriting height 3 and 4 is ok
            let result2 = da.fork_at(2, &[vec![3, 3, 3, 3], vec![4, 4, 4, 4]]).await;
            assert!(result2.is_ok());
            let block_2_after_reorg = da.get_block_at(2).await?;
            let block_3_after_reorg = da.get_block_at(3).await?;

            assert_eq!(block_2_after, block_2_after_reorg);
            assert_ne!(block_3_after, block_3_after_reorg);

            Ok(())
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test_planned_reorg() -> anyhow::Result<()> {
            let mut da = MockDaService::new(MockAddress::new([1; 32])).with_finality(4);
            da.wait_attempts = 2;

            // Planned for will replace blocks at height 3 and 4
            let planned_fork = PlannedFork::new(4, 2, vec![vec![3, 3, 3, 3], vec![4, 4, 4, 4]]);

            da.set_planned_fork(planned_fork).await?;
            assert!(da.planned_fork.is_some());

            da.send_transaction(&[1, 2, 3, 4]).await.await??;
            da.send_transaction(&[4, 5, 6, 7]).await.await??;
            da.send_transaction(&[8, 9, 0, 1]).await.await??;

            let block_1_before = da.get_block_at(1).await?;
            let block_2_before = da.get_block_at(2).await?;
            assert_consecutive_blocks(&block_1_before, &block_2_before);
            let block_3_before = da.get_block_at(3).await?;
            assert_consecutive_blocks(&block_2_before, &block_3_before);
            let block_4 = da.get_block_at(4).await?;

            // Fork is happening!
            assert_ne!(block_3_before.header().hash(), block_4.header().prev_hash());
            let block_3_after = da.get_block_at(3).await?;
            assert_consecutive_blocks(&block_3_after, &block_4);
            assert_consecutive_blocks(&block_2_before, &block_3_after);
            // Still have it, but it is old
            assert!(da.planned_fork.is_some());
            Ok(())
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test_planned_reorg_shorter() -> anyhow::Result<()> {
            let mut da = MockDaService::new(MockAddress::new([1; 32])).with_finality(4);
            da.wait_attempts = 2;
            // Planned for will replace blocks at height 3 and 4
            let planned_fork =
                PlannedFork::new(4, 2, vec![vec![13, 13, 13, 13], vec![14, 14, 14, 14]]);
            da.set_planned_fork(planned_fork).await?;

            da.send_transaction(&[1, 1, 1, 1]).await.await??;
            da.send_transaction(&[2, 2, 2, 2]).await.await??;
            da.send_transaction(&[3, 3, 3, 3]).await.await??;
            da.send_transaction(&[4, 4, 4, 4]).await.await??;
            da.send_transaction(&[5, 5, 5, 5]).await.await??;

            let block_1_before = da.get_block_at(1).await?;
            let block_2_before = da.get_block_at(2).await?;
            assert_consecutive_blocks(&block_1_before, &block_2_before);
            let block_3_before = da.get_block_at(3).await?;
            assert_consecutive_blocks(&block_2_before, &block_3_before);
            let block_4 = da.get_block_at(4).await.unwrap();
            assert_ne!(block_4.header().prev_hash(), block_3_before.header().hash());
            let block_1_after = da.get_block_at(1).await?;
            let block_2_after = da.get_block_at(2).await?;
            let block_3_after = da.get_block_at(3).await?;
            assert_consecutive_blocks(&block_3_after, &block_4);
            assert_consecutive_blocks(&block_2_after, &block_3_after);
            assert_consecutive_blocks(&block_1_after, &block_2_after);

            let block_5_result = da.get_block_at(5).await;
            assert!(block_5_result
                .unwrap_err()
                .to_string()
                .starts_with("No block at height=5 has been sent in "));
            Ok(())
        }
    }

    fn assert_consecutive_blocks(block1: &MockBlock, block2: &MockBlock) {
        assert_eq!(block2.header().prev_hash(), block1.header().hash());
    }
}
