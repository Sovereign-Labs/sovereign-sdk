mod config;
mod ledger_rpc;

#[cfg(test)]
mod test_rpc;
mod txs_rpc;

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use demo_stf::app::{
    DefaultContext, DefaultPrivateKey, DemoBatchReceipt, DemoTxReceipt, NativeAppRunner,
};
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runner_config::from_toml_path;
use demo_stf::runtime::{get_rpc_methods, GenesisConfig};
use jsonrpsee::core::server::rpc_module::Methods;
use presence::service::DaProvider as AvailDaProvider;
use risc0_adapter::host::Risc0Verifier;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_modules_api::RpcRunner;
use sov_rollup_interface::crypto::NoOpHasher;
use sov_rollup_interface::da::{BlobTransactionTrait, DaVerifier};
use sov_rollup_interface::services::da::{DaService, SlotData};
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_state::Storage;
use tracing::{debug, info, Level};

use crate::config::RollupConfig;
// RPC related imports
use crate::txs_rpc::get_txs_rpc;

pub fn initialize_ledger(path: impl AsRef<std::path::Path>) -> LedgerDB {
    LedgerDB::with_path(path).expect("Ledger DB failed to open")
}

async fn start_rpc_server(methods: impl Into<Methods>, address: SocketAddr) {
    let server = jsonrpsee::server::ServerBuilder::default()
        .build([address].as_ref())
        .await
        .unwrap();
    let _server_handle = server.start(methods).unwrap();
    futures::future::pending::<()>().await;
}

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

//TODO: Add validity checker?

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());
    debug!("Starting demo rollup with config {}", rollup_config_path);
    let rollup_config: RollupConfig =
        from_toml_path(&rollup_config_path).context("Failed to read rollup configuration")?;
    let rpc_config = rollup_config.rpc_config;
    let address = SocketAddr::new(rpc_config.bind_host.parse()?, rpc_config.bind_port);

    // Initializing logging
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    // Initialize the ledger database, which stores blocks, transactions, events, etc.
    let ledger_db = initialize_ledger(&rollup_config.runner.storage.path);

    let node_client = presence::build_client(rollup_config.da.node_client_url.to_string(), false)
        .await
        .unwrap();
    let light_client_url = rollup_config.da.light_client_url.to_string();
    // Initialize the Avail service using the DaService interface
    let da_service = Arc::new(AvailDaProvider {
        node_client,
        light_client_url,
    });

    // Our state transition function implements the StateTransitionRunner interface, so we use that to intitialize the STF
    let mut demo_runner = NativeAppRunner::<Risc0Verifier>::new(rollup_config.runner.clone());

    // Our state transition also implements the RpcRunner interface, so we use that to initialize the RPC server.
    let storage = demo_runner.get_storage();
    let is_storage_empty = storage.is_empty();
    let mut methods = get_rpc_methods(storage);
    let ledger_rpc_module =
        ledger_rpc::get_ledger_rpc::<DemoBatchReceipt, DemoTxReceipt>(ledger_db.clone());
    methods
        .merge(ledger_rpc_module)
        .expect("Failed to merge rpc modules");

    let batch_builder = demo_runner.take_batch_builder().unwrap();

    let txs_rpc = get_txs_rpc(batch_builder, da_service.clone());

    methods
        .merge(txs_rpc)
        .expect("Failed to merge Txs RPC modules");

    let _handle = tokio::spawn(async move {
        start_rpc_server(methods, address).await;
    });

    // For demonstration,  we also intitalize the DaVerifier interface using the DaVerifier interface
    // Running the verifier is only *necessary* during proof generation not normal execution
    let da_verifier = presence::verifier::Verifier {};

    let demo = demo_runner.inner_mut();
    let mut prev_state_root = {
        // Check if the rollup has previously been initialized
        if is_storage_empty {
            info!("No history detected. Initializing chain...");
            demo.init_chain(get_genesis_config(&rollup_config.sequencer_da_address));
            info!("Chain initialization is done.");
        } else {
            debug!("Chain is already initialized. Skipping initialization.");
        }

        // HACK: Tell the rollup that you're running an empty DA layer block so that it will return the latest state root.
        // This will be removed shortly.
        demo.begin_slot(Default::default());
        let (prev_state_root, _) = demo.end_slot();
        prev_state_root.0
    };

    // Start the main rollup loop
    let item_numbers = ledger_db.get_next_items_numbers();
    let last_slot_processed_before_shutdown = item_numbers.slot_number - 1;
    let start_height = rollup_config.start_height + last_slot_processed_before_shutdown;

    for height in start_height.. {
        info!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(prev_state_root)
        );

        // Fetch the relevant subset of the next Avail block
        let filtered_block = da_service.get_finalized_at(height).await?;
        let header = filtered_block.header().clone();

        // For the demo, we create and verify a proof that the data has been extracted from Avail correctly.
        // The inclusion and completeness proof in the case is not verified by the adapter, but the light client is trusted to have
        // verified it already.
        let (blob_txs, inclusion_proof, completeness_proof) =
            da_service.extract_relevant_txs_with_proof(&filtered_block);
        assert!(da_verifier
            .verify_relevant_tx_list::<NoOpHasher>(
                &header,
                &blob_txs,
                inclusion_proof,
                completeness_proof
            )
            .is_ok());
        info!("Received {} blobs", blob_txs.len());

        demo.begin_slot(Default::default());
        let mut data_to_commit = SlotCommit::new(filtered_block);
        for blob in &mut blob_txs.clone() {
            info!("sender: {}", hex::encode(blob.sender()));
            let receipts = demo.apply_blob(blob, None);
            info!("er: {:?}", receipts);
            data_to_commit.add_batch(receipts);
        }
        let (next_state_root, _witness) = demo.end_slot();

        // Store the resulting receipts in the ledger database
        ledger_db.commit_slot(data_to_commit)?;
        prev_state_root = next_state_root.0;
    }

    Ok(())
}
