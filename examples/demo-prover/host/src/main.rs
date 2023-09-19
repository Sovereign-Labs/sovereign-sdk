use std::env;
use std::str::FromStr;

use anyhow::Context;
use const_rollup_config::{ROLLUP_NAMESPACE_RAW, SEQUENCER_DA_ADDRESS};
use demo_stf::app::App;
use demo_stf::genesis_config::get_genesis_config;
use methods::{ROLLUP_ELF, ROLLUP_ID};
use sov_celestia_adapter::types::NamespaceId;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::verifier::{CelestiaSpec, RollupParams};
use sov_celestia_adapter::{CelestiaService, DaServiceConfig};
use sov_modules_api::SlotData;
use sov_risc0_adapter::host::{Risc0Host, Risc0Verifier};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;
use sov_state::Storage;
use sov_stf_runner::{from_toml_path, RollupConfig};
use tracing::{info, Level};

// The rollup stores its data in the namespace b"sov-test" on Celestia
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId(ROLLUP_NAMESPACE_RAW);

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

    // Same rollup_config.toml as used for the demo_rollup
    // When running from the demo-prover folder, the first argument can be pointed to ../demo-rollup/rollup_config.toml
    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());
    let rollup_config: RollupConfig<DaServiceConfig> =
        from_toml_path(&rollup_config_path).context("Failed to read rollup configuration")?;

    // New Celestia DA service to fetch blocks from the DA node (light client / docker / mock DA)
    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    )
    .await;

    let mut app: App<Risc0Verifier, CelestiaSpec> = App::new(rollup_config.storage.clone());

    let is_storage_empty = app.get_storage().is_empty();

    // If storage is empty, we're starting from scratch, so we need to initialize
    if is_storage_empty {
        let sequencer_da_address = CelestiaAddress::from_str(SEQUENCER_DA_ADDRESS).unwrap();
        let genesis_config = get_genesis_config(sequencer_da_address.as_ref().to_vec()).genesis;
        info!("Starting from empty storage, initialization chain");
        app.stf.init_chain(genesis_config);
    }

    let mut prev_state_root = app
        .get_storage()
        .get_state_root(&Default::default())
        .expect("The storage needs to have a state root");

    // We start from the height in rollup_config. When running with docker, this is usually height 1
    for height in rollup_config.runner.start_height.. {
        // We initialize a new VM with the rollup ELF.
        // ROLLUP_ELF points to the riscV ELF code generated by the risc0 infrastructure
        // Risc0Host::new carries out the process of compiling the code in methods/guest/src/bin/rollup.rs
        // and generating the ELF file. (The risc0 code builds a new toolchain to enable compiling to a riscV llvm backend)
        let mut host = Risc0Host::new(ROLLUP_ELF);
        // This function is used to communicate to the rollup.rs code running inside the VM
        // The reads need to be in order of the writes
        // prev_state_root is the root after applying the block at height-1
        // This is necessary since we're proving that the current state root for the current height is
        // result of applying the block against state with root prev_state_root
        host.add_hint(prev_state_root);
        info!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(prev_state_root)
        );
        let filtered_block = da_service.get_finalized_at(height).await?;
        let header_hash = hex::encode(filtered_block.header.header.hash());
        host.add_hint(&filtered_block.header);
        // When we get a block from DA, we also need to provide proofs of completeness and correctness
        // https://github.com/Sovereign-Labs/sovereign-sdk/blob/nightly/rollup-interface/specs/interfaces/da.md#type-inclusionmultiproof
        let (mut blobs, inclusion_proof, completeness_proof) = da_service
            .extract_relevant_txs_with_proof(&filtered_block)
            .await;

        info!(
            "Extracted {} relevant blobs at height {} header 0x{}",
            blobs.len(),
            height,
            header_hash,
        );

        // The above proofs of correctness and completeness need to passed to the prover
        host.add_hint(&inclusion_proof);
        host.add_hint(&completeness_proof);

        let result = app.stf.apply_slot(
            Default::default(),
            filtered_block.header(),
            &filtered_block.validity_condition(),
            &mut blobs,
        );

        // The extracted blobs need to be passed to the prover after execution.
        // (Without executing, the host couldn't prune any data that turned out to be irrelevant to the guest)
        host.add_hint(&blobs);

        // Witness contains the merkle paths to the state root so that the code inside the VM
        // can access state values (Witness can also contain other hints and proofs)
        host.add_hint(&result.witness);

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

        // Set the value of prev_state_root to the current one in preparation for the next block
        prev_state_root = result.state_root.0;
        info!("Completed proving and verifying block {height}");
    }

    Ok(())
}
