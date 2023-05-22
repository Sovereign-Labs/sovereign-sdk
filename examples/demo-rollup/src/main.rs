mod config;

use crate::config::RollupConfig;
use demo_stf::app::create_demo_genesis_config;
use demo_stf::app::get_rpc_module;
use demo_stf::app::{DefaultPrivateKey, NativeAppRunner};
use demo_stf::config::from_toml_path;
use jsonrpsee::RpcModule;
use jupiter::da_service::CelestiaService;
use jupiter::types::NamespaceId;
use jupiter::verifier::CelestiaVerifier;
use jupiter::verifier::RollupParams;
use risc0_adapter::host::Risc0Host;
use sovereign_core::da::DaVerifier;
use sovereign_core::services::da::DaService;
use sovereign_core::stf::{StateTransitionFunction, StateTransitionRunner};
use sovereign_db::ledger_db::{LedgerDB, SlotCommit};
use std::net::SocketAddr;
use tracing::Level;

const DATA_DIR_LOCATION: &str = "demo_data";
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId([115, 111, 118, 45, 116, 101, 115, 116]);

pub fn initialize_ledger() -> LedgerDB {
    let ledger_db = LedgerDB::with_path(DATA_DIR_LOCATION).expect("Ledger DB failed to open");
    ledger_db
}

async fn rpc(module: RpcModule<()>, address: SocketAddr) {
    let server = jsonrpsee::server::ServerBuilder::default()
        .build([address].as_ref())
        .await
        .unwrap();
    let _server_handle = server.start(module).unwrap();
    futures::future::pending::<()>().await;
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let rollup_config: RollupConfig = from_toml_path("rollup_config.toml")?;
    let rpc_config = rollup_config.rpc_config;
    let address = SocketAddr::new(rpc_config.bind_host.parse()?, rpc_config.bind_port);

    // Initializing logging
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::WARN)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    let ledger_db = initialize_ledger();

    // Initialize the Celestia service
    let mut demo_runner = NativeAppRunner::<Risc0Host>::new(rollup_config.runner.clone());

    let storj = demo_runner.inner().current_storage.clone();
    let module = get_rpc_module(storj);

    let _handle = tokio::spawn(async move {
        rpc(module, address).await;
    });

    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    );
    let da_verifier = CelestiaVerifier::new(RollupParams {
        namespace: ROLLUP_NAMESPACE,
    });

    // Initialize the demo app
    let demo = demo_runner.inner_mut();
    let sequencer_private_key = DefaultPrivateKey::generate();
    let genesis_config = create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        vec![
            99, 101, 108, 101, 115, 116, 105, 97, 49, 113, 112, 48, 57, 121, 115, 121, 103, 99,
            120, 54, 110, 112, 116, 101, 100, 53, 121, 99, 48, 97, 117, 54, 107, 57, 108, 110, 101,
            114, 48, 53, 121, 118, 115, 57, 50, 48, 56,
        ],
        &sequencer_private_key,
        &sequencer_private_key,
    );
    println!("priv: {}", sequencer_private_key.as_hex());

    let item_numbers = ledger_db.get_next_items_numbers();
    let last_slot_processed_before_shutdown = item_numbers.slot_number - 1;
    if last_slot_processed_before_shutdown == 0 {
        print!("No history detected. Initializing chain...");
        demo.init_chain(genesis_config);
        println!("Done.");
    } else {
        println!("Chain is already initialized. Skipping initialization.");
    }

    demo.begin_slot(Default::default());
    let (prev_state_root, _, _) = demo.end_slot();
    let mut prev_state_root = prev_state_root.0;

    let start_height = rollup_config.start_height + last_slot_processed_before_shutdown;

    for height in start_height.. {
        println!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(&prev_state_root)
        );
        let filtered_block = da_service.get_finalized_at(height).await?;
        let header = filtered_block.header.clone();
        let (blob_txs, inclusion_proof, completeness_proof) =
            da_service.extract_relevant_txs_with_proof(filtered_block.clone());
        assert!(da_verifier
            .verify_relevant_tx_list(&header, &blob_txs, inclusion_proof, completeness_proof)
            .is_ok());
        println!("Received {} blobs", blob_txs.len());

        let mut data_to_commit = SlotCommit::new(filtered_block);

        demo.begin_slot(Default::default());
        for blob in blob_txs.clone() {
            let receipts = demo.apply_blob(blob, None);
            println!("er: {:?}", receipts);
            data_to_commit.add_batch(receipts);
        }
        let (next_state_root, _witness, _) = demo.end_slot();
        ledger_db.commit_slot(data_to_commit)?;
        prev_state_root = next_state_root.0;
    }

    println!("waiting on RPC");
    futures::future::pending::<()>().await;

    Ok(())
}
