//! Implementation of a Data Availability service that supports storing its data in a database.
//! Currently, SQLite and PostgreSQL are supported.
//!

mod entity;
pub mod layer;
pub mod service;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::task::Poll;

    use futures::future::poll_fn;
    use futures::FutureExt;
    use proptest::collection::vec;
    use proptest::prelude::*;
    use sov_rollup_interface::da::{BlobReaderTrait, Time};
    use sov_rollup_interface::node::da::DaService;
    use tokio::sync::RwLock;

    use crate::storable::layer::StorableMockDaLayer;
    use crate::storable::service::StorableMockDaService;
    use crate::{BlockProducingConfig, MockAddress, MockDaConfig};

    #[tokio::test(flavor = "multi_thread")]
    async fn manually_triggered_blocks_are_fetched_after_await() -> anyhow::Result<()> {
        // This test checks that if `get_block_at` has been called before `produce_block`,
        // caller of `get_block_at` will get the new block.
        let config = MockDaConfig {
            connection_string: MockDaConfig::sqlite_in_memory(),
            sender_address: Default::default(),
            finalization_blocks: 0,
            block_producing: BlockProducingConfig::Manual,
            da_layer: None,
            randomization: None,
        };
        let blocks = 5;
        let start = Time::now();

        let (_shutdown_sender, mut shutdown_receiver) = tokio::sync::watch::channel(());
        shutdown_receiver.mark_unchanged();

        let da_service = StorableMockDaService::from_config(config, shutdown_receiver).await;
        let da_service_reader = da_service.clone();

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        // First, start a reader task.
        // It is important that it goes first,
        // so we can really check if it receives a block that has been triggered after it started to wait
        let receiver_task = tokio::task::spawn(async move {
            let mut block_times = Vec::with_capacity(blocks);
            for h in 1..=blocks {
                let mut fut = da_service_reader.get_block_at(h as u64);
                poll_fn(|cx| match fut.poll_unpin(cx) {
                    Poll::Pending => Poll::Ready(true),
                    Poll::Ready(_) => {
                        panic!("Get block should not be ready at this moment");
                    }
                })
                .await;
                tx.send(()).await.unwrap();
                let result = fut.await;
                block_times.push(result.unwrap().header.time);
            }
            block_times
        });

        while tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await?
            .is_some()
        {
            da_service.produce_block_now().await?;
        }
        let end = Time::now();

        let block_times =
            tokio::time::timeout(std::time::Duration::from_secs(10), receiver_task).await??;

        assert_eq!(block_times.len(), blocks);
        for block_time in block_times {
            assert!(
                block_time >= start,
                "Block time {block_time:?} is before start {start:?} of the da service"
            );
            assert!(
                block_time <= end,
                "Block time {block_time:?} is after last producing block {end:?}"
            );
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn manually_produce_blocks_from_different_sender_with_timestamp() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let timestamp = Time::from_millis(100001);

        let da_layer = Arc::new(RwLock::new(
            StorableMockDaLayer::new_in_path(tempdir.path(), 0).await?,
        ));

        let sender_1 = MockAddress::new([0; 32]);
        let da_service_1 =
            StorableMockDaService::new_manual_producing(sender_1, da_layer.clone()).await;
        let sender_2 = MockAddress::new([1; 32]);
        let da_service_2 =
            StorableMockDaService::new_manual_producing(sender_2, da_layer.clone()).await;

        let blob_0_0 = vec![0, 0];
        let blob_0_1 = vec![0, 0];
        let blob_1_0 = vec![0, 0];

        let _ = da_service_1.send_transaction(&blob_0_0).await.await??;
        let _ = da_service_2.send_transaction(&blob_1_0).await.await??;
        let _ = da_service_1.send_transaction(&blob_0_1).await.await??;

        {
            let mut layer = da_layer.write().await;
            layer
                .produce_block_with_timestamp(timestamp.clone())
                .await?;
        }

        let mut block = da_service_1.get_block_at(1).await?;
        assert_eq!(block.header.time, timestamp);
        assert_eq!(3, block.batch_blobs.len());

        let expected_data = vec![
            (sender_1, blob_0_1.clone()),
            (sender_2, blob_1_0.clone()),
            (sender_1, blob_0_1.clone()),
        ];

        for (idx, (expected_sender, data)) in expected_data.into_iter().enumerate() {
            let blob = &mut block.batch_blobs[idx];
            assert_eq!(blob.address, expected_sender);
            let actual_full_data = blob.full_data();
            assert_eq!(actual_full_data, &data);
        }

        Ok(())
    }

    #[derive(
        Debug, Clone, Copy, Eq, Hash, PartialEq, proptest_derive::Arbitrary, arbitrary::Arbitrary,
    )]
    // More than enough senders for decentralized DA.
    enum TestDaSender {
        One,
        Two,
        Three,
    }

    impl TestDaSender {
        fn address(&self) -> MockAddress {
            match self {
                TestDaSender::One => MockAddress::new([0; 32]),
                TestDaSender::Two => MockAddress::new([1; 32]),
                TestDaSender::Three => MockAddress::new([2; 32]),
            }
        }

        async fn build_da_services(
            da_layer: Arc<RwLock<StorableMockDaLayer>>,
        ) -> HashMap<TestDaSender, StorableMockDaService> {
            let mut da_services: HashMap<TestDaSender, StorableMockDaService> = HashMap::new();
            for sender in [TestDaSender::One, TestDaSender::Two, TestDaSender::Three] {
                da_services.insert(
                    sender,
                    StorableMockDaService::new_manual_producing(sender.address(), da_layer.clone())
                        .await,
                );
            }

            da_services
        }

        fn build_blob_data(&self, idx: u8) -> Vec<u8> {
            let mut blob = vec![0, idx];
            blob[0] = match self {
                TestDaSender::One => 0,
                TestDaSender::Two => 1,
                TestDaSender::Three => 2,
            };
            blob
        }
    }

    #[derive(Debug, Clone, proptest_derive::Arbitrary, arbitrary::Arbitrary)]
    enum BlobType {
        Batch,
        Proof,
    }

    type BlockDesign = Vec<(TestDaSender, BlobType)>;
    type ChainDesign = Vec<BlockDesign>;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_manual_block_production_simple() -> anyhow::Result<()> {
        let chain = vec![
            vec![
                (TestDaSender::One, BlobType::Batch),
                (TestDaSender::Three, BlobType::Proof),
                (TestDaSender::One, BlobType::Batch),
            ],
            vec![
                (TestDaSender::Two, BlobType::Batch),
                (TestDaSender::One, BlobType::Proof),
                (TestDaSender::One, BlobType::Batch),
            ],
        ];

        test_chain_design(chain).await
    }

    /// Test first submits batches or proofs according to given ChainDesign
    /// and then validates that correct data is available via `get_block_at`
    async fn test_chain_design(chain: ChainDesign) -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let da_layer = Arc::new(RwLock::new(
            StorableMockDaLayer::new_in_path(tempdir.path(), 0).await?,
        ));

        let da_services = TestDaSender::build_da_services(da_layer.clone()).await;
        // Indexes are needed to produce different blobs from the same senders.
        let mut batch_indexes: HashMap<TestDaSender, u8> = HashMap::new();
        let mut proof_indexes: HashMap<TestDaSender, u8> = HashMap::new();

        for (idx, block_design) in chain.iter().enumerate() {
            for (sender, blob_type) in block_design {
                let da_service = da_services.get(sender).unwrap();
                match blob_type {
                    BlobType::Batch => {
                        let batch_idx = batch_indexes.entry(*sender).or_insert(0);
                        let blob = sender.build_blob_data(*batch_idx);
                        da_service.send_transaction(&blob).await.await??;
                        *batch_idx += 1;
                    }
                    BlobType::Proof => {
                        let proof_idx = proof_indexes.entry(*sender).or_insert(0);
                        let blob = sender.build_blob_data(*proof_idx);
                        da_service.send_proof(&blob).await.await??;
                        *proof_idx += 1;
                    }
                }
            }
            let mut layer = da_layer.write().await;
            let timestamp = Time::from_secs(idx as i64);
            layer
                .produce_block_with_timestamp(timestamp.clone())
                .await?;
        }

        // Submission is done. Now validating!
        let mut batch_indexes: HashMap<TestDaSender, u8> = HashMap::new();
        let mut proof_indexes: HashMap<TestDaSender, u8> = HashMap::new();
        let da_service = da_services.get(&TestDaSender::One).unwrap();
        for (idx, block_design) in chain.iter().enumerate() {
            let height = idx as u64 + 1;
            let mut block = da_service.get_block_at(height).await?;

            let expected_timestamp = Time::from_secs(idx as i64);
            assert_eq!(block.header.time, expected_timestamp);

            let mut expected_batches = Vec::new();
            let mut expected_proofs = Vec::new();
            for (sender, blob_type) in block_design {
                match blob_type {
                    BlobType::Batch => {
                        let batch_idx = batch_indexes.entry(*sender).or_insert(0);
                        let blob = sender.build_blob_data(*batch_idx);
                        expected_batches.push((*sender, blob));
                        *batch_idx += 1;
                    }
                    BlobType::Proof => {
                        let proof_idx = proof_indexes.entry(*sender).or_insert(0);
                        let blob = sender.build_blob_data(*proof_idx);
                        expected_proofs.push((*sender, blob));
                        *proof_idx += 1;
                    }
                };
            }

            assert_eq!(expected_batches.len(), block.batch_blobs.len());
            assert_eq!(expected_proofs.len(), block.proof_blobs.len());

            // Validate batches
            for ((expected_sender, expected_blob), blob) in
                expected_batches.iter().zip(block.batch_blobs.iter_mut())
            {
                assert_eq!(blob.address, expected_sender.address());
                let actual_full_data = blob.full_data();
                assert_eq!(actual_full_data, &expected_blob[..]);
            }
            // Validate proofs
            for ((expected_sender, expected_blob), blob) in
                expected_proofs.iter().zip(block.proof_blobs.iter_mut())
            {
                assert_eq!(blob.address, expected_sender.address());
                let actual_full_data = blob.full_data();
                assert_eq!(actual_full_data, &expected_blob[..]);
            }
        }

        Ok(())
    }

    // Assuming the original type definitions are in scope
    prop_compose! {
        // Generate a single block design with a reasonable number of entries.
        fn block_design_strategy()
            (entries in vec((any::<TestDaSender>(), any::<BlobType>()), 0..30))
             -> BlockDesign {
            entries
        }
    }

    prop_compose! {
        // Generate a chain design with a reasonable length.
        fn chain_design_strategy()
            (blocks in vec(block_design_strategy(), 1..10))
             -> ChainDesign {
            blocks
        }
    }

    proptest! {
        #[test]
        fn proptest_manual_block_production(chain in chain_design_strategy()) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                test_chain_design(chain).await.unwrap();
            });
        }
    }
}
