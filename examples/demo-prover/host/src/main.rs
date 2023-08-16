use std::env;
use std::str::FromStr;

use anyhow::Context;
use const_rollup_config::{ROLLUP_NAMESPACE_RAW, SEQUENCER_DA_ADDRESS};
use demo_stf::app::{App, DefaultPrivateKey};
use demo_stf::genesis_config::create_demo_genesis_config;
use jupiter::da_service::{CelestiaService, DaServiceConfig};
use jupiter::types::NamespaceId;
use jupiter::verifier::address::CelestiaAddress;
use jupiter::verifier::{ChainValidityCondition, RollupParams};
use methods::{ROLLUP_ELF, ROLLUP_ID};
use risc0_adapter::host::{Risc0Host, Risc0Verifier};
use serde::Deserialize;
use sov_modules_api::PrivateKey;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;
use sov_state::Storage;
use sov_stf_runner::{from_toml_path, Config as RunnerConfig};
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
    let skip_prover = env::var("SKIP_PROVER").is_ok();
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
    )
    .await;

    let sequencer_private_key = DefaultPrivateKey::generate();

    let mut app: App<Risc0Verifier, ChainValidityCondition, jupiter::BlobWithSender> =
        App::new(rollup_config.runner.storage.clone());

    let is_storage_empty = app.get_storage().is_empty();

    if is_storage_empty {
        let sequencer_da_address = CelestiaAddress::from_str(SEQUENCER_DA_ADDRESS).unwrap();
        let genesis_config = create_demo_genesis_config(
            100000000,
            sequencer_private_key.default_address(),
            sequencer_da_address.as_ref().to_vec(),
            &sequencer_private_key,
            &sequencer_private_key,
        );
        info!("Starting from empty storage, initialization chain");
        app.stf.init_chain(genesis_config);
    }

    let mut prev_state_root = app
        .get_storage()
        .get_state_root(&Default::default())
        .expect("The storage needs to have a state root");

    for height in rollup_config.start_height.. {
        let mut host = Risc0Host::new(ROLLUP_ELF);
        host.write_to_guest(prev_state_root);
        info!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(prev_state_root)
        );
        let filtered_block = da_service.get_finalized_at(height).await?;
        let header_hash = hex::encode(filtered_block.header.header.hash());
        host.write_to_guest(&filtered_block.header);
        let (mut blobs, inclusion_proof, completeness_proof) = da_service
            .extract_relevant_txs_with_proof(&filtered_block)
            .await;

        info!(
            "Extracted {} relevant blobs at height {} header 0x{}",
            blobs.len(),
            height,
            header_hash,
        );

        host.write_to_guest(&inclusion_proof);
        host.write_to_guest(&completeness_proof);
        host.write_to_guest(&blobs);

        let result = app
            .stf
            .apply_slot(Default::default(), &filtered_block, &mut blobs);

        host.write_to_guest(&result.witness);

        if !skip_prover {
            info!("Starting proving...");
            let receipt = host.run().expect("Prover should run successfully");
            info!("Start verifying..");
            receipt.verify(ROLLUP_ID).expect("Receipt should be valid");
        } else {
            let _receipt = host
                .run_without_proving()
                .expect("Prover should run successfully");
        }

        prev_state_root = result.state_root.0;
        info!("Completed proving and verifying block {height}");
    }

    Ok(())
}
