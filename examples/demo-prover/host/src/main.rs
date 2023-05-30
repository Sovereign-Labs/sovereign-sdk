use anyhow::Context;
use const_rollup_config::{ROLLUP_NAMESPACE_RAW, SEQUENCER_DA_ADDRESS};
use demo_stf::app::{DefaultPrivateKey, NativeAppRunner};
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runner_config::from_toml_path;
use demo_stf::runner_config::Config as RunnerConfig;
use jupiter::da_service::{CelestiaService, DaServiceConfig};
use jupiter::types::NamespaceId;
use jupiter::verifier::RollupParams;
use methods::{ROLLUP_ELF, ROLLUP_ID};
use risc0_adapter::host::Risc0Host;
use serde::Deserialize;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::{StateTransitionFunction, StateTransitionRunner};
use sov_rollup_interface::zk::traits::ZkvmHost;
use std::env;

use tracing::{info, Level};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RollupConfig {
    pub start_height: u64,
    pub da: DaServiceConfig,
    pub runner: RunnerConfig,
}

// The rollup stores its data in the namespace b"sov-test" on Celestia
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId(ROLLUP_NAMESPACE_RAW);

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Initializing logging
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());
    let rollup_config: RollupConfig =
        from_toml_path(&rollup_config_path).context("Failed to read rollup configuration")?;

    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    );

    let sequencer_private_key = DefaultPrivateKey::generate();
    let genesis_config = create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        SEQUENCER_DA_ADDRESS.to_vec(),
        &sequencer_private_key,
        &sequencer_private_key,
    );

    let mut demo_runner = NativeAppRunner::<Risc0Host>::new(rollup_config.runner.clone());
    let demo = demo_runner.inner_mut();

    demo.init_chain(genesis_config);

    demo.begin_slot(Default::default());
    let (prev_state_root, _) = demo.end_slot();
    let mut prev_state_root = prev_state_root.0;

    for height in rollup_config.start_height..=rollup_config.start_height + 30 {
        let mut host = Risc0Host::new(ROLLUP_ELF);
        info!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(prev_state_root)
        );
        let filtered_block = da_service.get_finalized_at(height).await?;
        let header_hash = hex::encode(filtered_block.header.header.hash());
        host.write_to_guest(&filtered_block.header);
        let (blob_txs, inclusion_proof, completeness_proof) =
            da_service.extract_relevant_txs_with_proof(filtered_block);

        host.write_to_guest(&blob_txs);
        host.write_to_guest(&inclusion_proof);
        host.write_to_guest(&completeness_proof);
        host.write_to_guest(prev_state_root);

        demo.begin_slot(Default::default());
        if blob_txs.is_empty() {
            info!(
                "Block at height {} with header 0x{} has no batches, skip proving",
                height, header_hash
            );
            continue;
        }
        info!("Block has {} batches", blob_txs.len());
        for blob in blob_txs.clone() {
            let receipt = demo.apply_blob(blob, None);
            info!(
                "batch with hash=0x{} has been applied",
                hex::encode(receipt.batch_hash)
            );
        }

        let (next_state_root, witness) = demo.end_slot();
        host.write_to_guest(&witness);

        info!("Starting proving...");
        let receipt = host.run().expect("Prover should run successfully");
        info!("Start verifying..");
        receipt.verify(&ROLLUP_ID).expect("Receipt should be valid");

        prev_state_root = next_state_root.0;
        info!("Completed proving and verifying block {height}");
    }

    Ok(())
}
