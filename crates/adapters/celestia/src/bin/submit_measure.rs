#![allow(clippy::float_arithmetic)]
use backon::ExponentialBuilder;
use celestia_types::nmt::Namespace;
use clap::{Parser, ValueEnum};
use rand::Rng;
use serde::Deserialize;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::{CelestiaConfig, CelestiaService, TwinkleClient, TwinkleConfig};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Parser, Debug)]
#[command(name = "submit_measure")]
#[command(about = "Measure Celestia submission performance", long_about = None)]
struct Args {
    /// Path to the config file
    #[arg(value_name = "PATH")]
    path: PathBuf,

    /// Adapter to use for submission
    #[arg(short, long, value_enum)]
    adapter: Adapter,
}

#[derive(Debug, Clone, Deserialize)]
struct Payload {
    #[serde(skip_deserializing)]
    #[allow(dead_code)]
    blob_bytes: usize,
    max_batch_size_bytes: usize,
    max_concurrent_blobs: usize,
    batch_namespace: String,
    proof_namespace: String,
}

#[derive(Debug, Clone, Deserialize)]
struct VanillaConfig {
    #[allow(dead_code)]
    da: CelestiaConfig,
    payload: Payload,
}

#[derive(Debug, Clone, Deserialize)]
struct TwinkleConfigWrapper {
    da: TwinkleConfig,
    payload: Payload,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Adapter {
    Vanilla,
    Twinkle,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let config_str = std::fs::read_to_string(&args.path)?;

    let result = match args.adapter {
        Adapter::Vanilla => {
            let config: VanillaConfig = toml::from_str(&config_str)?;

            let rollup_params = RollupParams {
                rollup_batch_namespace: Namespace::const_v0(
                    config.payload.batch_namespace.as_bytes().try_into()?,
                ),
                rollup_proof_namespace: Namespace::const_v0(
                    config.payload.proof_namespace.as_bytes().try_into()?,
                ),
            };
            let service = CelestiaService::new(config.da, rollup_params).await;

            // Run measurement for 5 minutes
            let measurement_duration = Duration::from_secs(5 * 60);
            measure_throughput_vanilla(
                service,
                config.payload.max_batch_size_bytes,
                config.payload.max_concurrent_blobs,
                measurement_duration,
            )
            .await?
        }
        Adapter::Twinkle => {
            let config: TwinkleConfigWrapper = toml::from_str(&config_str)?;

            let namespace =
                Namespace::const_v0(config.payload.batch_namespace.as_bytes().try_into()?);

            // Initialize TwinkleClient
            let backoff_policy = ExponentialBuilder::default();
            let client = TwinkleClient::from_config(&config.da, backoff_policy)?;

            // Run measurement for 5 minutes
            let measurement_duration = Duration::from_secs(5 * 60);
            measure_throughput_twinkle(
                client,
                namespace,
                config.payload.max_batch_size_bytes,
                config.payload.max_concurrent_blobs,
                measurement_duration,
            )
            .await?
        }
    };

    result.print_report();

    Ok(())
}

async fn measure_throughput_vanilla(
    service: CelestiaService,
    blob_size: usize,
    max_concurrent: usize,
    duration: Duration,
) -> anyhow::Result<MeasurementResult> {
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let total_blobs = Arc::new(AtomicUsize::new(0));
    let total_bytes = Arc::new(AtomicUsize::new(0));
    let failed_blobs = Arc::new(AtomicUsize::new(0));

    // Get starting block height
    let start_header = service.get_head_block_header().await?;
    let start_height = start_header.height();

    println!(
        "Starting measurement (blob_size={blob_size} bytes, max_concurrent={max_concurrent}, duration={duration:?})..."
    );

    let start = Instant::now();

    while start.elapsed() < duration {
        let permit = semaphore.clone().acquire_owned().await?;
        let service = service.clone();
        let total_blobs = total_blobs.clone();
        let total_bytes = total_bytes.clone();
        let failed_blobs = failed_blobs.clone();

        // Generate random blob data before spawning
        let mut rng = rand::thread_rng();
        let blob: Vec<u8> = (0..blob_size).map(|_| rng.gen::<u8>()).collect();
        let blob_bytes = blob.len();

        tokio::spawn(async move {
            // Submit blob
            let rx = service.send_transaction(&blob).await;
            match rx.await {
                Ok(Ok(_receipt)) => {
                    total_blobs.fetch_add(1, Ordering::Relaxed);
                    total_bytes.fetch_add(blob_bytes, Ordering::Relaxed);
                }
                Ok(Err(e)) => {
                    eprintln!("Blob submission failed: {e:?}");
                    failed_blobs.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    eprintln!("Failed to receive submission result: {e:?}");
                    failed_blobs.fetch_add(1, Ordering::Relaxed);
                }
            }

            drop(permit);
        });
    }

    println!("Measurement duration complete. Not waiting for in-flight blobs.");

    // Get ending block height
    let end_header = service.get_head_block_header().await?;
    let end_height = end_header.height();
    println!("Ending height: {end_height}");

    let elapsed = start.elapsed();
    let blobs_count = total_blobs.load(Ordering::Relaxed);
    let bytes_count = total_bytes.load(Ordering::Relaxed);
    let failed_count = failed_blobs.load(Ordering::Relaxed);

    Ok(MeasurementResult {
        adapter: Adapter::Vanilla,
        duration: elapsed,
        total_blobs: blobs_count,
        total_bytes: bytes_count,
        failed_blobs: failed_count,
        start_height,
        end_height,
    })
}

async fn measure_throughput_twinkle(
    client: TwinkleClient,
    namespace: Namespace,
    blob_size: usize,
    max_concurrent: usize,
    duration: Duration,
) -> anyhow::Result<MeasurementResult> {
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let total_blobs = Arc::new(AtomicUsize::new(0));
    let total_bytes = Arc::new(AtomicUsize::new(0));
    let failed_blobs = Arc::new(AtomicUsize::new(0));

    // Get starting block height
    let start_header = client.get_head_block_header().await?;
    let start_height: u64 = start_header.height();
    println!("Starting height: {start_height}");

    let start = Instant::now();
    let min_interval = Duration::from_millis(4000);
    let mut last_submission = Instant::now();

    println!(
        "Starting measurement: blob_size={blob_size} bytes, max_concurrent={max_concurrent}, duration={duration:?}"
    );
    println!("Rate limiting: minimum {min_interval:?} between submission attempts");

    while start.elapsed() < duration {
        // Rate limiting: ensure at least 300ms between submission attempts
        let elapsed_since_last = last_submission.elapsed();
        if elapsed_since_last < min_interval {
            tokio::time::sleep(min_interval - elapsed_since_last).await;
        }
        last_submission = Instant::now();

        let permit = semaphore.clone().acquire_owned().await?;
        let client = client.clone();
        let total_blobs = total_blobs.clone();
        let total_bytes = total_bytes.clone();
        let failed_blobs = failed_blobs.clone();

        // Generate random blob data before spawning
        let mut rng = rand::thread_rng();
        let blob: Vec<u8> = (0..blob_size).map(|_| rng.gen::<u8>()).collect();
        let blob_bytes = blob.len();

        tokio::spawn(async move {
            // Submit blob (using internal method that TwinkleClient has)
            let rx = client
                .submit_blob_to_namespace_inner(&blob, namespace)
                .await;
            match rx.await {
                Ok(Ok(_receipt)) => {
                    total_blobs.fetch_add(1, Ordering::Relaxed);
                    total_bytes.fetch_add(blob_bytes, Ordering::Relaxed);
                }
                Ok(Err(e)) => {
                    eprintln!("Blob submission failed: {e:?}");
                    failed_blobs.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    eprintln!("Failed to receive submission result: {e:?}");
                    failed_blobs.fetch_add(1, Ordering::Relaxed);
                }
            }

            drop(permit);
        });
    }

    println!("Measurement duration complete. Not waiting for in-flight blobs.");

    // Get ending block height
    let end_header = client.get_head_block_header().await?;
    let end_height: u64 = end_header.height();
    println!("Ending height: {end_height}");

    let elapsed = start.elapsed();
    let blobs_count = total_blobs.load(Ordering::Relaxed);
    let bytes_count = total_bytes.load(Ordering::Relaxed);
    let failed_count = failed_blobs.load(Ordering::Relaxed);

    Ok(MeasurementResult {
        adapter: Adapter::Twinkle,
        duration: elapsed,
        total_blobs: blobs_count,
        total_bytes: bytes_count,
        failed_blobs: failed_count,
        start_height,
        end_height,
    })
}

struct MeasurementResult {
    adapter: Adapter,
    duration: Duration,
    total_blobs: usize,
    total_bytes: usize,
    failed_blobs: usize,
    start_height: u64,
    end_height: u64,
}

impl MeasurementResult {
    fn print_report(&self) {
        let duration_secs = self.duration.as_secs_f64();
        let throughput_kibs = (self.total_bytes as f64) / 1024.0 / duration_secs;
        let blocks_produced = self.end_height.saturating_sub(self.start_height);
        let total_attempts = self.total_blobs + self.failed_blobs;
        let success_rate = if total_attempts > 0 {
            (self.total_blobs as f64 / total_attempts as f64) * 100.0
        } else {
            0.0
        };
        let landing_ratio = if blocks_produced > 0 {
            (self.total_blobs as f64 / blocks_produced as f64) * 100.0
        } else {
            0.0
        };

        println!("\n=== Measurement Results ===");
        println!("Adapter: {:?}", self.adapter);
        println!("Duration: {duration_secs:.2}s");
        println!("Start height: {}", self.start_height);
        println!("End height: {}", self.end_height);
        println!("Blocks produced: {blocks_produced}");
        println!();
        println!("Submission attempts: {total_attempts}");
        println!("Successful submissions: {}", self.total_blobs);
        println!("Failed submissions: {}", self.failed_blobs);
        println!("Success rate: {success_rate:.2}%");
        println!();
        println!("Blob landing ratio: {landing_ratio:.2}% (blobs per block)");
        println!("Total bytes submitted: {} bytes", self.total_bytes);
        println!("Throughput: {throughput_kibs:.2} KiB/s");
        if self.total_blobs > 0 {
            println!(
                "Average blob size: {} bytes",
                self.total_bytes / self.total_blobs
            );
            println!(
                "Successful blobs per second: {:.2}",
                self.total_blobs as f64 / duration_secs
            );
        }
        println!("==========================\n");
    }
}
