use std::env;
use std::str::FromStr;

use anyhow::Context;
use const_rollup_config::SEQUENCER_AVAIL_DA_ADDRESS;
use demo_stf::app::{App, DefaultContext, DefaultPrivateKey};
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runtime::GenesisConfig;
use methods::{ROLLUP_ELF, ROLLUP_ID};
use presence::service::{DaProvider, DaServiceConfig};
use presence::spec::transaction::AvailBlobTransaction;
use presence::spec::address::AvailAddress;
use presence::spec::DaLayerSpec;
use risc0_adapter::host::{Risc0Host, Risc0Verifier};
use sov_modules_api::PrivateKey;
use sov_rollup_interface::services::da::{DaService, SlotData};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;
use sov_state::Storage;
use sov_stf_runner::{from_toml_path, RollupConfig};
use tracing::{info, Level};

pub fn get_genesis_config(
    sequencer_da_address: &AvailAddress,
) -> GenesisConfig<DefaultContext, DaLayerSpec> {
    let sequencer_private_key = DefaultPrivateKey::generate();

    create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        sequencer_da_address.as_ref().to_vec(),
        &sequencer_private_key,
    )
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // If SKIP_PROVER is set, this means that we still compile and generate the riscV ELF
    // We execute the code inside the riscV but we don't prove it. This saves a significant amount of time
    // The primary benefit of doing this is to make sure we produce valid code that can run inside the
    // riscV virtual machine. Since proving is something we offload entirely to risc0, ensuring that
    // we produce valid riscV code and that it can execute is very useful.
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
    let rollup_config: RollupConfig<DaServiceConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;

    let da_service = DaProvider::new(rollup_config.da.clone()).await;

    let mut app: App<Risc0Verifier, DaLayerSpec> = App::new(rollup_config.storage);

    let is_storage_empty = app.get_storage().is_empty();
    
    let sequencer_da_address = AvailAddress::from_str(SEQUENCER_AVAIL_DA_ADDRESS)?;
    if is_storage_empty {
        info!("Starting from empty storage, initialization chain");
        app.stf
            .init_chain(get_genesis_config(&sequencer_da_address));
    }

    let mut prev_state_root = app
        .get_storage()
        .get_state_root(&Default::default())
        .expect("The storage needs to have a state root");

    for height in rollup_config.runner.start_height.. {
        let mut host = Risc0Host::new(ROLLUP_ELF);
        host.write_to_guest(prev_state_root);

        info!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(prev_state_root)
        );
        let filtered_block = da_service.get_finalized_at(height).await?;
        let header_hash = hex::encode(filtered_block.hash());
        host.write_to_guest(&filtered_block.header);
        let (mut blob_txs, inclusion_proof, completeness_proof) = da_service
            .extract_relevant_txs_with_proof(&filtered_block)
            .await;

        info!(
            "Extracted {} relevant blobs at height {} header 0x{}",
            blob_txs.len(),
            height,
            header_hash,
        );

        host.write_to_guest(&inclusion_proof);
        host.write_to_guest(&completeness_proof);
        host.write_to_guest(&blob_txs);

        let result = app.stf.apply_slot(Default::default(), &filtered_block, &mut blob_txs);

        host.write_to_guest(&result.witness);

        // Run the actual prover to generate a receipt that can then be verified
        if !skip_prover {
            info!("Starting proving...");
            let receipt = host.run().expect("Prover should run successfully");
            info!("Start verifying..");
            receipt.verify(ROLLUP_ID).expect("Receipt should be valid");
        } else {
            // This runs the riscV code inside the VM without actually generating the proofs
            // This is useful for testing if rollup code actually executes properly
            let _receipt = host
                .run_without_proving()
                .expect("Prover should run successfully");
        }

        prev_state_root = result.state_root.0;
        info!("Completed proving and verifying block {height}");
    }

    Ok(())
}
