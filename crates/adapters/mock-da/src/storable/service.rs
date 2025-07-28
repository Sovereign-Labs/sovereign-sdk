//! Data Availability service is a controller of [`StorableMockDaLayer`].
use core::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{
    BlobReaderTrait, BlockHeaderTrait, DaSpec, RelevantBlobs, RelevantProofs,
};
use sov_rollup_interface::node::da::{DaService, SlotData, SubmitBlobReceipt};
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use tokio::sync::{broadcast, oneshot, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep};
use tracing::Instrument;

use crate::config::WAIT_ATTEMPT_PAUSE;
use crate::storable::layer::{Randomizer, StorableMockDaLayer};
use crate::{
    BlockProducingConfig, MockAddress, MockBlock, MockBlockHeader, MockDaConfig, MockDaSpec,
    MockDaVerifier, RandomizationBehaviour, RandomizationConfig, DEFAULT_BLOCK_WAITING_TIME_MS,
};

const DEFAULT_BLOCK_WAITING_TIME: Duration = Duration::from_secs(3600);
// Time to accommodate rare cases of lock waiting time or latency to the database.
const EXTRA_TIME_FOR_MAX_BLOCK: Duration = Duration::from_secs(10);

impl BlockProducingConfig {
    fn get_max_waiting_time_for_block(&self) -> Duration {
        match self {
            // Use a large number to prevent infinite blocking.
            BlockProducingConfig::Manual => DEFAULT_BLOCK_WAITING_TIME,
            BlockProducingConfig::Periodic { block_time_ms } => {
                Duration::from_millis(*block_time_ms) + EXTRA_TIME_FOR_MAX_BLOCK
            }
            BlockProducingConfig::OnBatchSubmit {
                block_wait_timeout_ms,
            }
            | BlockProducingConfig::OnAnySubmit {
                block_wait_timeout_ms,
            } => {
                Duration::from_millis(
                    block_wait_timeout_ms.unwrap_or(DEFAULT_BLOCK_WAITING_TIME_MS),
                ) + EXTRA_TIME_FOR_MAX_BLOCK
            }
        }
    }

    /// Spawns periodic block producing. Useful for testing or custom setup.
    fn spawn_block_producing_if_needed(
        &self,
        shutdown_receiver: watch::Receiver<()>,
        da_layer: Arc<RwLock<StorableMockDaLayer>>,
    ) -> Option<JoinHandle<()>> {
        let BlockProducingConfig::Periodic { block_time_ms } = self else {
            return None;
        };

        let block_time = Duration::from_millis(*block_time_ms);
        let span = tracing::info_span!("periodic_batch_producer");

        Some(tokio::spawn(
            async move {
                tracing::debug!(interval = ?block_time, "Spawning a task for periodic producing");
                loop {
                    match future_or_shutdown(tokio::time::sleep(block_time), &shutdown_receiver)
                        .await
                    {
                        FutureOrShutdownOutput::Shutdown => {
                            tracing::debug!(
                                "Received shutdown signal, stopping block production..."
                            );
                            break;
                        }
                        FutureOrShutdownOutput::Output(_) => {
                            let mut da_layer = da_layer.write().await;
                            if let Err(error) = da_layer.produce_block().await {
                                tracing::warn!(
                                    ?error,
                                    "Error producing new block. Will try next time."
                                );
                            }
                        }
                    }
                }
                tracing::info!("Periodic block producing is stopped");
            }
            .instrument(span),
        ))
    }
}

/// Allows consuming the [`futures::Stream`] of BlockHeaders.
type HeaderStream = BoxStream<'static, Result<MockBlockHeader, anyhow::Error>>;

/// DaService that works on top of [`StorableMockDaLayer`].
#[derive(Clone)]
pub struct StorableMockDaService {
    /// The address of the sequencer.
    pub sequencer_da_address: MockAddress,
    da_layer: Arc<RwLock<StorableMockDaLayer>>,
    block_producing: BlockProducingConfig,
    aggregated_proof_sender: broadcast::Sender<()>,
    head_block: watch::Receiver<MockBlockHeader>,
    block_producer_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    block_producing_pauser: Arc<Mutex<Option<watch::Sender<()>>>>,
    send_transaction_success: Arc<AtomicBool>,
}

impl StorableMockDaService {
    async fn construct(
        sequencer_da_address: MockAddress,
        da_layer: Arc<RwLock<StorableMockDaLayer>>,
        block_producing: BlockProducingConfig,
        block_producer_handle: Option<JoinHandle<()>>,
    ) -> Self {
        let (aggregated_proof_subscription, mut rec) = broadcast::channel(16);
        tokio::spawn(async move { while rec.recv().await.is_ok() {} });
        let head_block = {
            let da_layer = da_layer.read().await;
            da_layer.subscribe_to_head_updates()
        };

        Self {
            sequencer_da_address,
            da_layer,
            block_producing,
            aggregated_proof_sender: aggregated_proof_subscription,
            head_block,
            block_producer_handle: Arc::new(Mutex::new(block_producer_handle)),
            block_producing_pauser: Arc::new(Mutex::new(None)),
            send_transaction_success: Arc::new(AtomicBool::new(true)),
        }
    }
    /// The `send_transaction` method will fail to post blobs to the DA.
    pub fn set_fail_send_blob(&self) {
        self.send_transaction_success
            .store(false, Ordering::Relaxed);
    }

    /// Returns a handle to the `StorableMockDaLayer` backing this service.
    pub fn da_layer(&self) -> Arc<RwLock<StorableMockDaLayer>> {
        self.da_layer.clone()
    }

    /// Returns the `BlockProducingConfig` used by this service
    pub fn block_producing(&self) -> &BlockProducingConfig {
        &self.block_producing
    }

    /// The `send_transaction` method will start posting blobs to the DA.
    pub fn set_success_send_blob(&self) {
        self.send_transaction_success.store(true, Ordering::Relaxed);
    }

    /// Suspend blob submission in the mock DA.
    pub async fn set_blob_submission_pause(&self) {
        let (sender, _) = watch::channel(());
        *self.block_producing_pauser.lock().await = Some(sender);
    }

    /// Resume blob submission in the mock DA.
    pub async fn resume_blob_submission(&self) {
        let mut sender = self.block_producing_pauser.lock().await;
        sender.as_ref().unwrap().send(()).unwrap();
        *sender = None;
    }

    /// Create a new [` StorableMockDaService `] with the given address.
    pub async fn new(
        sequencer_da_address: MockAddress,
        da_layer: Arc<RwLock<StorableMockDaLayer>>,
        block_producing: BlockProducingConfig,
    ) -> Self {
        if !matches!(block_producing, BlockProducingConfig::Periodic { .. }) {
            tracing::warn!("Periodic block should be spawned separately, please use Self::from_config otherwise");
        }
        Self::construct(sequencer_da_address, da_layer, block_producing, None).await
    }

    /// Create a new [` StorableMockDaService `] with the given address and [`BlockProducingConfig::Manual`].
    /// Shorter constructor.
    pub async fn new_manual_producing(
        sequencer_da_address: MockAddress,
        da_layer: Arc<RwLock<StorableMockDaLayer>>,
    ) -> Self {
        Self::new(sequencer_da_address, da_layer, BlockProducingConfig::Manual).await
    }

    /// Set the number of blocks to wait before including blobs on DA
    pub async fn set_delay_blobs_by(&self, delay: u32) {
        let mut da_layer = self.da_layer.write().await;
        da_layer.set_delay_blobs_by(delay);
    }

    /// Creates a new instance with different address, but on the same [`StorableMockDaLayer`].
    /// Block production of this new instance is manual.
    /// Panics if passed address is the same as the original one.
    pub async fn another_on_the_same_layer(&self, new_da_address: MockAddress) -> Self {
        if new_da_address == self.sequencer_da_address {
            panic!("DA address equal self, just call .clone()");
        }
        let da_layer = self.da_layer.clone();
        Self::new_manual_producing(new_da_address, da_layer).await
    }

    /// Will receive notification one block before the proof is included on the DA.
    pub fn subscribe_proof_posted(&self) -> broadcast::Receiver<()> {
        self.aggregated_proof_sender.subscribe()
    }

    /// Creates new [`StorableMockDaService`] with a given address.
    /// Block producing happens on blob submission.
    /// Data is stored only in memory.
    /// It is very similar to [`crate::MockDaService`] parameters.
    pub async fn new_in_memory(sequencer_da_address: MockAddress, blocks_to_finality: u32) -> Self {
        let da_layer = StorableMockDaLayer::new_in_memory(blocks_to_finality)
            .await
            .expect("Failed to initialize StorableMockDaLayer");
        let producing = BlockProducingConfig::OnBatchSubmit {
            block_wait_timeout_ms: None,
        };
        Self::new(
            sequencer_da_address,
            Arc::new(RwLock::new(da_layer)),
            producing,
        )
        .await
    }

    /// Creates new in memory [`StorableMockDaService`] from [`MockDaConfig`].
    pub async fn from_config(config: MockDaConfig, shutdown_receiver: watch::Receiver<()>) -> Self {
        let da_layer = match config.da_layer.as_ref() {
            None => {
                let mut da_layer = StorableMockDaLayer::new_from_connection(
                    &config.connection_string,
                    config.finalization_blocks,
                )
                .await
                .expect("Failed to initialize StorableMockDaLayer");
                if let Some(randomization) = &config.randomization {
                    tracing::debug!(
                        config = ?randomization,
                        "StorableMockDaLayer will have randomizer"
                    );
                    da_layer.set_randomizer(Randomizer::from_config(randomization.clone()));
                }
                Arc::new(RwLock::new(da_layer))
            }
            Some(da_layer) => da_layer.clone(),
        };
        let handle = config
            .block_producing
            .spawn_block_producing_if_needed(shutdown_receiver, da_layer.clone());
        Self::construct(
            config.sender_address,
            da_layer,
            config.block_producing,
            handle,
        )
        .await
    }

    async fn wait_for_height(&self, height: u32) -> anyhow::Result<()> {
        let start_wait = Instant::now();
        let max_waiting_time = self.block_producing.get_max_waiting_time_for_block();
        let mut interval = interval(WAIT_ATTEMPT_PAUSE);

        loop {
            tokio::select! {
                // self.head_block.changed() requires &mut self
                // But at least we aren't touching rw lock to shared layer.
                // It can be wrapped in Arc<RwLock> too
                _ = interval.tick() => {
                    // current height is height of currently building block.
                    let current_head_height = {
                        self.head_block.borrow().height as u32
                    };

                    // Head can be queried.
                    if current_head_height >= height {
                        return Ok(())
                    }
                }
                _ = sleep(max_waiting_time.saturating_sub(start_wait.elapsed())) => {
                    anyhow::bail!("No block at height={height} has been sent in {:?}", max_waiting_time);
                }
            }
        }
    }

    /// Trigger creation of a new block on underlying [`StorableMockDaLayer`].
    pub async fn produce_block_now(&self) -> anyhow::Result<()> {
        let mut da_layer = self.da_layer.write().await;
        da_layer.produce_block().await
    }

    /// Wrapper around [`StorableMockDaService::produce_block_now`] to quickly
    /// advance the DA by a number of blocks.
    ///
    /// This is especially useful at the beginning of tests, to "maximize" the
    /// finalization distance between genesis and DA chain head.
    pub async fn produce_n_blocks_now(&self, n: usize) -> anyhow::Result<()> {
        let mut da_layer = self.da_layer.write().await;
        for _ in 0..n {
            da_layer.produce_block().await?;
        }
        Ok(())
    }

    /// Sets randomized blob retrieval by adjust [`Randomizer`] in underlying [`StorableMockDaLayer`].
    /// Passing None disables randomization.
    /// Passing Some enables or changes randomization to be out of order on retrieval.
    pub async fn set_randomized_blobs_retrieval(&self, seed: Option<[u8; 32]>) {
        let mut da_layer = self.da_layer.write().await;
        match seed {
            Some(seed) => {
                let finality = da_layer.blocks_to_finality;
                da_layer.set_randomizer(Randomizer::from_config(RandomizationConfig {
                    seed: HexHash::new(seed),
                    // Not really applicable for this scenario, but still put something sensible.
                    reorg_interval: 1..finality,
                    behaviour: RandomizationBehaviour::OutOfOrderBlobs,
                }));
            }
            None => {
                let _ = da_layer.disable_randomizer();
            }
        }
    }

    /// Re-org simulation: Rewinds the underlying [`StorableMockDaLayer`] to the specified height.
    /// Refer to [`StorableMockDaLayer::rewind_to_height`] for more details.
    pub async fn rewind_to_height(&self, height: u32) -> anyhow::Result<()> {
        let mut da_layer = self.da_layer.write().await;
        da_layer.rewind_to_height(height).await
    }

    /// Subscribe to finalized headers as they are finalized.
    /// Expect only to receive headers which were finalized after subscription
    /// Optimized version of `get_last_finalized_block_header`.
    pub async fn subscribe_finalized_header(&self) -> Result<HeaderStream, anyhow::Error> {
        let receiver = {
            let da_layer = self.da_layer.read().await;
            da_layer.finalized_header_sender.subscribe()
        };

        let stream = futures::stream::unfold(receiver, |mut receiver| async move {
            match receiver.recv().await {
                Ok(header) => Some((Ok(header), receiver)),
                Err(_) => None,
            }
        });

        Ok(stream.boxed())
    }
}

#[async_trait]
impl DaService for StorableMockDaService {
    type Spec = MockDaSpec;
    type Config = MockDaConfig;
    type Verifier = MockDaVerifier;
    type FilteredBlock = MockBlock;
    type Error = anyhow::Error;

    const GUARANTEES_TRANSACTION_ORDERING: bool = true;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        tracing::trace!(%height, "Getting block at");
        if height > u32::MAX as u64 {
            return Err(anyhow::anyhow!(
                "Height {} is too big for StorableMockDaService. Max is {}",
                height,
                u32::MAX
            ));
        }

        let height = height as u32;

        self.wait_for_height(height).await?;

        let block = {
            let da_layer = self.da_layer.read().await;
            da_layer.get_block_at(height).await?
        };

        tracing::trace!(block_header = %block.header().display(), "Block retrieved");
        Ok(block)
    }

    async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        tracing::trace!(%height, "Getting block header at");
        if height > u32::MAX as u64 {
            return Err(anyhow::anyhow!(
                "Height {} is too big for StorableMockDaService. Max is {}",
                height,
                u32::MAX
            ));
        }

        let height = height as u32;
        let block_header = {
            // TODO: What if future ?
            let da_layer = self.da_layer.read().await;
            da_layer.get_block_header_at(height).await?
        };

        tracing::trace!(block_header = %block_header.display(), "Block header retrieved");
        Ok(block_header)
    }

    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        self.da_layer
            .read()
            .await
            .get_last_finalized_block_header()
            .await
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
        if !self.send_transaction_success.load(Ordering::Relaxed) {
            tx.send(Err(anyhow::anyhow!(
                "StorableMockDaService::send_transaction failed"
            )))
            .unwrap();
            return rx;
        }

        let block_producing_pauser = {
            let pauser = self.block_producing_pauser.lock().await;
            pauser.clone()
        };

        if let Some(sender) = block_producing_pauser {
            let mut rec = sender.subscribe();
            rec.changed().await.unwrap();
        }

        let should_produce_block = match &self.block_producing {
            BlockProducingConfig::OnBatchSubmit { .. }
            | BlockProducingConfig::OnAnySubmit { .. } => true,
            BlockProducingConfig::Periodic { .. } | BlockProducingConfig::Manual => false,
        };
        tracing::trace!(batch = hex::encode(blob), "Submitting a batch");
        let blob_hash = {
            let mut da_layer = self.da_layer.write().await;
            let blob_hash = da_layer
                .submit_batch(blob, &self.sequencer_da_address)
                .await
                .unwrap();
            tracing::trace!(%should_produce_block, "Batch has been sent to DA, producing block if necessary");
            if should_produce_block {
                da_layer.produce_block().await.unwrap();
            }
            blob_hash
        };
        let res = Ok(SubmitBlobReceipt {
            blob_hash: HexHash::new(blob_hash.0),
            da_transaction_id: blob_hash,
        });

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
        tracing::trace!(
            blob = hex::encode(aggregated_proof_data),
            "Sending an aggregated proof"
        );

        let should_produce_block = match &self.block_producing {
            BlockProducingConfig::OnBatchSubmit { .. } => {
                tracing::debug!("Proof submission won't produce new DA block");
                false
            }
            BlockProducingConfig::OnAnySubmit { .. } => true,
            BlockProducingConfig::Periodic { .. } | BlockProducingConfig::Manual => false,
        };

        let blob_hash = {
            let mut da_layer = self.da_layer.write().await;
            let blob_hash = da_layer
                .submit_proof(aggregated_proof_data, &self.sequencer_da_address)
                .await
                .unwrap();
            tracing::trace!(%should_produce_block, "Proof has been sent to DA, producing block if necessary");
            if should_produce_block {
                da_layer.produce_block().await.unwrap();
            }
            blob_hash
        };

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

    async fn take_background_join_handle(&self) -> Option<JoinHandle<()>> {
        self.block_producer_handle.lock().await.take()
    }

    async fn get_signer(&self) -> <Self::Spec as DaSpec>::Address {
        self.sequencer_da_address
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rand::Rng;

    use super::*;
    use crate::config::GENESIS_HEADER;

    async fn check_consistency(
        da_service: &StorableMockDaService,
        expected_blobs_count: usize,
    ) -> anyhow::Result<()> {
        let mut prev_block_hash = GENESIS_HEADER.prev_hash;

        let head_block = da_service.get_head_block_header().await?;
        {
            let da_layer = da_service.da_layer.read().await;
            let db_head_block = da_layer.get_head_block_header().await?;
            assert_eq!(head_block, db_head_block);
        }

        let mut total_blobs_count = 0;
        for height in 0..=head_block.height() {
            let block = da_service.get_block_at(height).await?;
            assert_eq!(height, block.header().height());
            assert_eq!(prev_block_hash, block.header().prev_hash());
            prev_block_hash = block.header().hash();

            total_blobs_count += block.batch_blobs.len() + block.proof_blobs.len();
        }

        assert_eq!(
            expected_blobs_count, total_blobs_count,
            "total blobs count do no match"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn multiple_threads_producing_reading() -> anyhow::Result<()> {
        let da_layer = Arc::new(RwLock::new(StorableMockDaLayer::new_in_memory(0).await?));
        let block_time = Duration::from_millis(50);
        let block_producing = BlockProducingConfig::Periodic {
            block_time_ms: block_time.as_millis() as u64,
        };

        let (shutdown_sender, mut shutdown_receiver) = tokio::sync::watch::channel(());
        shutdown_receiver.mark_unchanged();

        let producing_handle =
            block_producing.spawn_block_producing_if_needed(shutdown_receiver, da_layer.clone());

        let services_count = 20;
        let blobs_per_service = 50;

        let mut services_blobs: Vec<Vec<(Duration, Vec<u8>)>> = Vec::with_capacity(services_count);

        let mut rng = rand::thread_rng();
        for i in 0..services_count {
            let mut this_service_blobs = Vec::with_capacity(blobs_per_service);
            for j in 0..blobs_per_service {
                let blob = vec![(i as u8).saturating_mul(j as u8); 8];
                let sleep_time = Duration::from_millis(rng.gen_range(5..=40));
                this_service_blobs.push((sleep_time, blob));
            }
            services_blobs.push(this_service_blobs);
        }

        let mut handlers = Vec::new();
        for (idx, this_service_blobs) in services_blobs.into_iter().enumerate() {
            let this_da_layer = da_layer.clone();
            let this_block_producing = block_producing.clone();
            let address = MockAddress::new([idx as u8; 32]);
            handlers.push(tokio::spawn(async move {
                let da_service =
                    StorableMockDaService::new(address, this_da_layer, this_block_producing).await;
                for (wait, blob) in this_service_blobs {
                    sleep(wait).await;
                    da_service
                        .send_transaction(&blob)
                        .await
                        .await
                        .unwrap()
                        .unwrap();
                }
            }));
        }

        for handler in handlers {
            handler.await?;
        }
        // Sleep extra block time so all blocks are produced.
        sleep(block_time * 2).await;

        let da_service =
            StorableMockDaService::new(MockAddress::new([1; 32]), da_layer, block_producing).await;
        check_consistency(&da_service, services_count * blobs_per_service).await?;

        shutdown_sender.send(())?;
        drop(da_service);
        producing_handle.unwrap().await?;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn querying_height_above_u32_max() -> anyhow::Result<()> {
        let producing = BlockProducingConfig::OnBatchSubmit {
            block_wait_timeout_ms: Some(10),
        };
        let mut service = StorableMockDaService::new_in_memory(MockAddress::new([0; 32]), 0).await;
        service.block_producing = producing;

        let height_1 = u32::MAX as u64;
        let height_2 = u32::MAX as u64 + 1;

        let result_1 = service.get_block_at(height_1).await;
        assert!(result_1.is_err());
        let err = result_1.unwrap_err().to_string();
        assert_eq!("No block at height=4294967295 has been sent in 10.01s", err);

        let result_2 = service.get_block_at(height_2).await;
        assert!(result_2.is_err());
        let err = result_2.unwrap_err().to_string();
        assert_eq!(
            "Height 4294967296 is too big for StorableMockDaService. Max is 4294967295",
            err
        );

        Ok(())
    }
}
