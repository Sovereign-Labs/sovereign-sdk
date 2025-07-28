use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::prelude::SmallRng;
use rand::{Rng, SeedableRng};
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::MockDaConfig;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};

const BLOCK_TIME_MS: u64 = 50;
const READERS_COUNT: usize = 10;

/// Test emulates access patterns to [`StorableMockDaService`] in a regular rollup.
/// This means several independent tokio tasks:
///  - Periodical block production: 1 write() accessor
///  - Sequencer: Periodical batch submission: 1 write() accessor + data input
///  - ZK Manager: Periodical proof submission: 1 write() accessor + data input
///  - Runner and DA Sync: [`READERS_COUNT`] call `get_block_at` and `get_head_block_header()`
///    Note: Readers setup is deliberately aggressive to bring the worst case performance.
///    The real case is more modest: there's a couple of readers, and they read closer to head, sequentially.
fn bench_storable_mock_da_service(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    let temp = tempfile::tempdir().unwrap();

    let (sender, mut receiver) = tokio::sync::watch::channel(());
    receiver.mark_unchanged();

    let path = temp.path().join("mock-da.sqlite");

    let original_blocks = 1000;
    println!("Setting up MockDA and wait for {original_blocks} blocks to be produced");
    let (da_service, handles) = rt.block_on(async {
        let da_service = StorableMockDaService::from_config(
            MockDaConfig {
                connection_string: format!("sqlite://{}?mode=rwc", path.display()),
                sender_address: Default::default(),
                finalization_blocks: 0,
                block_producing: sov_mock_da::BlockProducingConfig::Periodic {
                    block_time_ms: BLOCK_TIME_MS,
                },
                da_layer: None,
                randomization: None,
            },
            receiver.clone(),
        )
        .await;

        let mut handles = vec![];

        handles.push(rt.spawn(spawn_send_transaction_task(
            da_service.clone(),
            receiver.clone(),
        )));
        handles.push(rt.spawn(spawn_send_proof_task(da_service.clone(), receiver.clone())));
        loop {
            let head = da_service.get_head_block_header().await.unwrap();
            if head.height() >= original_blocks {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(BLOCK_TIME_MS)).await;
        }
        // Starting readers, after some blocks are produced
        for _ in 0..READERS_COUNT {
            handles.push(rt.spawn(spawn_reader_task(da_service.clone(), receiver.clone())));
        }

        (da_service, handles)
    });

    let mut group = c.benchmark_group("StorableMockDaService");
    group.measurement_time(std::time::Duration::from_secs(90));

    // Early block small chain
    group.bench_function("get_block_at(42) with small chain", |b| {
        b.to_async(&rt).iter(|| async {
            // Perform your measured operation
            let block = da_service.get_block_at(42).await.unwrap();
            assert_eq!(42, block.header.height);
        });
    });

    // Wait fill up
    let medium_blocks = 5000;
    println!(
        "Going to wait for {medium_blocks} blocks to be produced for another set of measurements"
    );
    let head = rt.block_on(async {
        #[allow(unused_assignments)]
        let mut head_height = 0;
        loop {
            let head = da_service.get_head_block_header().await.unwrap();

            if head.height % 1000 == 0 {
                println!("Current head is {}...", head.height);
            }

            if head.height >= medium_blocks {
                head_height = head.height;
                println!("Filled up blocks, go back to measuring");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(BLOCK_TIME_MS)).await;
        }
        head_height
    });

    group.bench_function("get_block_at(42) with medium chain", |b| {
        b.to_async(&rt).iter(|| async {
            let block = da_service.get_block_at(42).await.unwrap();
            assert_eq!(42, block.header.height);
        });
    });

    // Late block
    group.bench_function("get_block_at(head) with medium chain", |b| {
        b.to_async(&rt).iter(|| async {
            let block = da_service.get_block_at(head).await.unwrap();
            assert_eq!(head, block.header.height);
        });
    });

    // Submission
    let data = vec![200; 3000];
    group.bench_function("submit_batch with medium chain", |b| {
        b.to_async(&rt).iter(|| async {
            let _s = black_box(
                da_service
                    .send_transaction(&data)
                    .await
                    .await
                    .unwrap()
                    .unwrap(),
            );
        });
    });

    group.finish();

    sender.send(()).unwrap();
    rt.block_on(async {
        for handle in handles {
            let _res = handle.await?;
        }
        Ok::<(), anyhow::Error>(())
    })
    .unwrap();
}

async fn spawn_send_transaction_task(
    da_service: StorableMockDaService,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
) -> anyhow::Result<()> {
    let mut rng = SmallRng::from_entropy();
    loop {
        let sleep_duration = tokio::time::Duration::from_millis(rng.gen_range(5..=BLOCK_TIME_MS));
        match future_or_shutdown(tokio::time::sleep(sleep_duration), &shutdown_receiver).await {
            FutureOrShutdownOutput::Shutdown => {
                break;
            }
            FutureOrShutdownOutput::Output(_) => {
                let size = rng.gen_range(1024..=30_000);
                let batch_data = vec![200_u8; size];
                let _s = da_service.send_transaction(&batch_data).await.await??;
            }
        }
    }
    Ok(())
}

async fn spawn_send_proof_task(
    da_service: StorableMockDaService,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
) -> anyhow::Result<()> {
    let mut rng = SmallRng::from_entropy();
    loop {
        let sleep_duration = tokio::time::Duration::from_millis(rng.gen_range(5..=BLOCK_TIME_MS));
        match future_or_shutdown(tokio::time::sleep(sleep_duration), &shutdown_receiver).await {
            FutureOrShutdownOutput::Shutdown => {
                break;
            }
            FutureOrShutdownOutput::Output(_) => {
                let size = rng.gen_range(1024..=30_000);
                let proof_data = vec![200_u8; size];
                let _s = da_service.send_proof(&proof_data).await.await??;
            }
        }
    }
    Ok(())
}

async fn spawn_reader_task(
    da_service: StorableMockDaService,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
) -> anyhow::Result<()> {
    let mut rng = SmallRng::from_entropy();
    loop {
        let head = da_service.get_head_block_header().await?.height;
        let block_to_query = rng.gen_range(1..=head);
        let sleep_duration =
            tokio::time::Duration::from_millis(rng.gen_range(1..=(BLOCK_TIME_MS / 3)));
        match future_or_shutdown(tokio::time::sleep(sleep_duration), &shutdown_receiver).await {
            FutureOrShutdownOutput::Shutdown => {
                break;
            }
            FutureOrShutdownOutput::Output(_) => {
                let _s = da_service.get_block_at(block_to_query).await?;
            }
        }
    }

    Ok(())
}

criterion_group!(benches, bench_storable_mock_da_service);
criterion_main!(benches);
