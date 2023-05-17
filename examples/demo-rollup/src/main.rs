mod config;

use crate::config::RollupConfig;
use demo_stf::app::create_demo_genesis_config;
use demo_stf::app::{DefaultPrivateKey, NativeAppRunner};
use demo_stf::config::from_toml_path;
use jupiter::da_service::CelestiaService;
use jupiter::types::NamespaceId;
use jupiter::verifier::CelestiaVerifier;
use jupiter::verifier::RollupParams;
use risc0_adapter::host::Risc0Host;
use sovereign_core::da::DaVerifier;
use sovereign_core::services::da::DaService;
use sovereign_core::stf::{StateTransitionFunction, StateTransitionRunner};
use sovereign_db::ledger_db::{LedgerDB, SlotCommit};
use tracing::Level;

// I sent 8 demo election transactions at height 293686, generated using the demo app data generator
const DATA_DIR_LOCATION: &str = "demo_data";
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId([115, 111, 118, 45, 116, 101, 115, 116]);

pub fn initialize_ledger() -> LedgerDB {
    let ledger_db = LedgerDB::with_path(DATA_DIR_LOCATION).expect("Ledger DB failed to open");
    ledger_db
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let rollup_config: RollupConfig = from_toml_path("rollup_config.toml")?;

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
            99, 101, 108, 101, 115, 116, 105, 97, 49, 122, 102, 118, 114, 114, 102, 97, 113, 57,
            117, 100, 54, 103, 57, 116, 52, 107, 122, 109, 115, 108, 112, 102, 50, 52, 121, 115,
            97, 120, 113, 102, 110, 122, 101, 101, 53, 119, 57,
        ],
        &sequencer_private_key,
        &sequencer_private_key,
    );

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
    // Request data from the DA layer and apply it to the demo app
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
            data_to_commit.add_batch(receipts);
        }
        let (next_state_root, _witness, _) = demo.end_slot();
        ledger_db.commit_slot(data_to_commit)?;
        prev_state_root = next_state_root.0;
    }

    Ok(())
}
