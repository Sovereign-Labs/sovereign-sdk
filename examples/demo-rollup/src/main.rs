mod config;

use crate::config::RollupConfig;
use anyhow::Context;
use const_rollup_config::{ROLLUP_NAMESPACE_RAW, SEQUENCER_DA_ADDRESS};
use demo_stf::app::DefaultContext;
use demo_stf::app::{DefaultPrivateKey, NativeAppRunner};
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runner_config::from_toml_path;
use demo_stf::runtime::GenesisConfig;
use jupiter::da_service::CelestiaService;
use jupiter::types::NamespaceId;
use jupiter::verifier::CelestiaVerifier;
use jupiter::verifier::RollupParams;
use risc0_adapter::host::Risc0Host;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_rollup_interface::da::DaVerifier;
use sov_rollup_interface::services::da::{DaService, SlotData};
use sov_rollup_interface::stf::{StateTransitionFunction, StateTransitionRunner};
use std::env;
use std::net::SocketAddr;
use tracing::Level;

// RPC related imports
use demo_stf::app::get_rpc_methods;
use jsonrpsee::RpcModule;
use sov_modules_api::RpcRunner;

// The rollup stores its data in the namespace b"sov-test" on Celestia
// You can change this constant to point your rollup at a different namespace
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId(ROLLUP_NAMESPACE_RAW);

pub fn initialize_ledger(path: impl AsRef<std::path::Path>) -> LedgerDB {
    LedgerDB::with_path(path).expect("Ledger DB failed to open")
}

async fn start_rpc_server(methods: RpcModule<()>, address: SocketAddr) {
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
/// If you want to customize the rollup to accept transactions from your own celestia
/// address, simply change the value of the SEQUENCER_DA_ADDRESS to your own address.
/// For example:
/// ```rust,no_run
/// const SEQUENCER_DA_ADDRESS: [u8;47] = *b"celestia1qp09ysygcx6npted5yc0au6k9lner05yvs9208"
/// ```
pub fn get_genesis_config() -> GenesisConfig<DefaultContext> {
    let sequencer_private_key = DefaultPrivateKey::generate();
    create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        SEQUENCER_DA_ADDRESS.to_vec(),
        &sequencer_private_key,
        &sequencer_private_key,
    )
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());
    let rollup_config: RollupConfig =
        from_toml_path(&rollup_config_path).context("Failed to read rollup configuration")?;
    let rpc_config = rollup_config.rpc_config;
    let address = SocketAddr::new(rpc_config.bind_host.parse()?, rpc_config.bind_port);

    // Initializing logging
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::WARN)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    // Initialize the ledger database, which stores blocks, transactions, events, etc.
    let ledger_db = initialize_ledger(&rollup_config.runner.storage.path);

    // Our state transition function implements the StateTransitionRunner interface, so we use that to intitialize the STF
    let mut demo_runner = NativeAppRunner::<Risc0Host>::new(rollup_config.runner.clone());

    // Our state transition also implements the RpcRunner interface, so we use that to initialize the RPC server.
    let storj = demo_runner.get_storage();
    let methods = get_rpc_methods(storj);

    let _handle = tokio::spawn(async move {
        start_rpc_server(methods, address).await;
    });

    // Initialize the Celestia service using the DaService interface
    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    );
    // For demonstration,  we also intitalize the DaVerifier interface using the DaVerifier interface
    // Running the verifier is only *necessary* during proof generation not normal execution
    let da_verifier = CelestiaVerifier::new(RollupParams {
        namespace: ROLLUP_NAMESPACE,
    });

    let demo = demo_runner.inner_mut();

    // Check if the rollup has previously processed any data. If not, run it's "genesis" initialization code
    let item_numbers = ledger_db.get_next_items_numbers();
    let last_slot_processed_before_shutdown = item_numbers.slot_number - 1;
    if last_slot_processed_before_shutdown == 0 {
        print!("No history detected. Initializing chain...");
        demo.init_chain(get_genesis_config());
        println!("Done.");
    } else {
        println!("Chain is already initialized. Skipping initialization.");
    }

    // HACK: Tell the rollup that you're running an empty DA layer block so that it will return the latest state root.
    // This will be removed shortly.
    demo.begin_slot(Default::default());
    let (prev_state_root, _, _) = demo.end_slot();
    let mut prev_state_root = prev_state_root.0;

    // Start the main rollup loop
    let start_height = rollup_config.start_height + last_slot_processed_before_shutdown;

    for height in start_height.. {
        println!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(prev_state_root)
        );

        // Fetch the relevant subset of the next Celestia block
        let filtered_block = da_service.get_finalized_at(height).await?;
        let header = filtered_block.header().clone();

        // For the demo, we create and verify a proof that the data has been extracted from Celestia correctly.
        // In a production implementation, this logic would only run on the prover node - regular full nodes could
        // simply download the data from Celestia without extracting and checking a merkle proof here,
        let (blob_txs, inclusion_proof, completeness_proof) =
            da_service.extract_relevant_txs_with_proof(filtered_block.clone());
        assert!(da_verifier
            .verify_relevant_tx_list(&header, &blob_txs, inclusion_proof, completeness_proof)
            .is_ok());
        println!("Received {} blobs", blob_txs.len());

        demo.begin_slot(Default::default());
        let mut data_to_commit = SlotCommit::new(filtered_block);
        for blob in blob_txs.clone() {
            let receipts = demo.apply_blob(blob, None);
            println!("er: {:?}", receipts);
            data_to_commit.add_batch(receipts);
        }
        let (next_state_root, _witness, _) = demo.end_slot();

        // Store the resulting receipts in the ledger database
        ledger_db.commit_slot(data_to_commit)?;
        prev_state_root = next_state_root.0;
    }

    Ok(())
}
