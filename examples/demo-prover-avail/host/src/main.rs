mod config;

use std::env;

use anyhow::Context;
use demo_stf::app::{DefaultContext, DefaultPrivateKey, NativeAppRunner};
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runner_config::{from_toml_path, Config as RunnerConfig};
use demo_stf::runtime::GenesisConfig;
use methods::{ROLLUP_ELF, ROLLUP_ID};
use presence::service::DaProvider as AvailDaProvider;
use risc0_adapter::host::Risc0Host;
use serde::Deserialize;
use sov_modules_api::RpcRunner;
use sov_rollup_interface::services::da::{DaService, SlotData};
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;
use sov_state::Storage;
use tracing::{debug, info, Level};

use crate::config::RollupConfig;

/// Configure our rollup with a centralized sequencer using the SEQUENCER_DA_ADDRESS
/// address constant. Since the centralize sequencer's address is consensus critical,
/// it has to be hardcoded as a constant, rather than read from the config at runtime.
///
/// If you want to customize the rollup to accept transactions from your own avail
/// address, simply change the value of the SEQUENCER_DA_ADDRESS to your own address.
/// For example:
/// ```rust,no_run
/// const SEQUENCER_DA_ADDRESS: &str = "d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d";
/// ```

pub fn get_genesis_config(sequencer_da_address: &str) -> GenesisConfig<DefaultContext> {
    let sequencer_private_key = DefaultPrivateKey::generate();
    create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        hex::decode(sequencer_da_address).unwrap(),
        &sequencer_private_key,
        &sequencer_private_key,
    )
}

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

    let node_client = presence::build_client(rollup_config.da.node_client_url.to_string(), false)
        .await
        .unwrap();
    let light_client_url = rollup_config.da.light_client_url.to_string();
    // Initialize the Avail service using the DaService interface
    let da_service = AvailDaProvider {
        node_client,
        light_client_url,
    };

    let mut demo_runner = NativeAppRunner::<Risc0Host>::new(rollup_config.runner.clone());
    let is_storage_empty = demo_runner.get_storage().is_empty();
    let demo = demo_runner.inner_mut();

    let mut prev_state_root = {
        // Check if the rollup has previously been initialized
        if is_storage_empty {
            info!("No history detected. Initializing chain...");
            demo.init_chain(get_genesis_config(&rollup_config.sequencer_da_address));
            info!("Chain initialization is done.");
        } else {
            info!("Chain is already initialized. Skipping initialization");
        }

        // HACK: Tell the rollup that you're running an empty DA layer block so that it will return the latest state root.
        // This will be removed shortly.
        demo.begin_slot(Default::default());
        let (prev_state_root, _) = demo.end_slot();
        info!("{:#?}", &prev_state_root.0);
        prev_state_root.0
    };

    //TODO: Start from slot processed before shut down.

    for height in rollup_config.start_height..=rollup_config.start_height + 30 {
        let mut host = Risc0Host::new(ROLLUP_ELF);
        info!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(prev_state_root)
        );

        let filtered_block = da_service.get_finalized_at(height).await?;
        let header_hash = hex::encode(filtered_block.hash());
        host.write_to_guest(&filtered_block.header);
        let (blob_txs, inclusion_proof, completeness_proof) =
            da_service.extract_relevant_txs_with_proof(&filtered_block);

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
        for mut blob in blob_txs.clone() {
            let receipt = demo.apply_blob(&mut blob, None);
            info!(
                "batch with hash=0x{} has been applied",
                hex::encode(receipt.batch_hash)
            );
        }

        let (next_state_root, witness) = demo.end_slot();
        host.write_to_guest(&witness);

        info!("Starting proving...");
        let receipt = host.run().unwrap();
        info!("Start verifying..");

        receipt.verify(&ROLLUP_ID).expect("Receipt should be valid");

        prev_state_root = next_state_root.0;
        info!("Completed proving and verifying block {height}");
    }

    Ok(())
}
