use backon::ExponentialBuilder;
use celestia_types::nmt::Namespace;
use clap::{Parser, ValueEnum};
use serde::Deserialize;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::{CelestiaConfig, CelestiaService, TwinkleClient, TwinkleConfig};
use std::path::PathBuf;
use sov_rollup_interface::node::da::{DaService, SlotData};

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

    println!("Processing config from: {}", args.path.display());
    println!("Using adapter: {:?}", args.adapter);

    let config_str = std::fs::read_to_string(&args.path)?;

    match args.adapter {
        Adapter::Vanilla => {
            let config: VanillaConfig = toml::from_str(&config_str)?;

            println!("Loaded CelestiaConfig");
            println!(
                "Payload: max_batch_size_bytes={}, max_concurrent_blobs={}",
                config.payload.max_batch_size_bytes, config.payload.max_concurrent_blobs
            );

            let rollup_params = RollupParams {
                rollup_batch_namespace: Namespace::const_v0(
                    config.payload.batch_namespace.as_bytes().try_into()?,
                ),
                rollup_proof_namespace: Namespace::const_v0(
                    config.payload.proof_namespace.as_bytes().try_into()?,
                ),
            };
            let service = CelestiaService::new(config.da, rollup_params).await;

            let head = service.get_head_block_header().await?;
            println!("Head: {:?}", head.header());
        }
        Adapter::Twinkle => {
            let config: TwinkleConfigWrapper = toml::from_str(&config_str)?;

            println!("Loaded TwinkleConfig");
            println!(
                "Payload: max_batch_size_bytes={}, max_concurrent_blobs={}",
                config.payload.max_batch_size_bytes, config.payload.max_concurrent_blobs
            );

            // Initialize TwinkleClient
            let backoff_policy = ExponentialBuilder::default();
            let _client = TwinkleClient::from_config(&config.da, backoff_policy)?;

            println!("Initialized TwinkleClient");
        }
    }

    // TODO: Implement submission measurement logic

    Ok(())
}
