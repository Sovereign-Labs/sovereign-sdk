#![allow(clippy::float_arithmetic)]
use celestia_types::nmt::Namespace;
use clap::Parser;
use rand::Rng;
use serde::Deserialize;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::{CelestiaConfig, CelestiaService};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tracing_subscriber::filter::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "submit_measure")]
#[command(about = "Measure Celestia submission performance", long_about = None)]
struct Args {
    /// Path to the DA config file (e.g., celestia_rollup_config.toml)
    #[arg(long, value_name = "PATH")]
    da_config_path: PathBuf,

    /// Path to the payload config file
    #[arg(long, value_name = "PATH")]
    payload_config_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
struct PayloadConfig {
    max_batch_size_bytes: usize,
    max_concurrent_blobs: usize,
    batch_namespace: String,
    proof_namespace: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DaConfig {
    da: CelestiaConfig,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize tracing subscriber with env filter (defaults to debug)
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new(
                "debug,tower=warn,hyper=warn,rustls=info,sov_metrics=error,reqwest=warn,",
            )
        }))
        .init();

    let da_config_str = std::fs::read_to_string(&args.da_config_path)?;
    let da_config: DaConfig = toml::from_str(&da_config_str)?;

    let with_twinkle = da_config.da.twinkle.is_some();
    println!("With Twinkle: {with_twinkle}");

    let payload_config_str = std::fs::read_to_string(&args.payload_config_path)?;
    let payload_config: PayloadConfig = toml::from_str(&payload_config_str)?;

    let rollup_params = RollupParams {
        rollup_batch_namespace: Namespace::const_v0(
            payload_config.batch_namespace.as_bytes().try_into()?,
        ),
        rollup_proof_namespace: Namespace::const_v0(
            payload_config.proof_namespace.as_bytes().try_into()?,
        ),
    };

    // Initialize CelestiaService which will internally choose type based on config
    let service = CelestiaService::new(da_config.da, rollup_params).await;

    // Run measurement for 5 minutes
    let measurement_duration = Duration::from_secs(5 * 60);
    let result = measure_throughput(
        service,
        payload_config.max_batch_size_bytes,
        payload_config.max_concurrent_blobs,
        measurement_duration,
    )
    .await?;

    result.print_report();

    Ok(())
}

async fn measure_throughput(
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

    let min_interval = Duration::from_millis(5000);
    let mut last_submission = Instant::now();

    println!(
        "Starting measurement (blob_size={blob_size} bytes, max_concurrent={max_concurrent}, duration={duration:?}) at height={start_height}..."
    );
    println!("Rate limiting: minimum {min_interval:?} between submission attempts");

    let start = Instant::now();
    while start.elapsed() < duration {
        // Rate limiting: ensure minimum interval between submission attempts
        let elapsed_since_last = last_submission.elapsed();
        if elapsed_since_last < min_interval {
            tokio::time::sleep(min_interval - elapsed_since_last).await;
        }
        last_submission = Instant::now();

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
            // Submit blob via DaService interface
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

    // Get ending block height
    let end_header = service.get_head_block_header().await?;
    let end_height: u64 = end_header.height();

    let elapsed = start.elapsed();
    let blobs_count = total_blobs.load(Ordering::Relaxed);
    let bytes_count = total_bytes.load(Ordering::Relaxed);
    let failed_count = failed_blobs.load(Ordering::Relaxed);

    Ok(MeasurementResult {
        duration: elapsed,
        total_blobs: blobs_count,
        total_bytes: bytes_count,
        failed_blobs: failed_count,
        start_height,
        end_height,
    })
}

struct MeasurementResult {
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
        println!("Duration: {duration_secs:.2}s");
        println!(
            "Produced {blocks_produced} blocks from {} to {}",
            self.start_height, self.end_height
        );
        println!();
        println!(
            "Submission: successful {}; failed {} out of {total_attempts}",
            self.total_blobs, self.failed_blobs
        );
        println!("Success rate: {success_rate:.2}%");
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
