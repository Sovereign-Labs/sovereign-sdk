mod config;
use std::env;
use std::sync::Arc;

use anyhow::Context;
use demo_stf::app::{
    DefaultContext, DefaultPrivateKey, App,
};
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runtime::{get_rpc_methods, GenesisConfig};
use presence::service::DaProvider as AvailDaProvider;
use presence::spec::transaction::AvailBlobTransaction;
use risc0_adapter::host::Risc0Verifier;
use sov_db::ledger_db::{LedgerDB};
use sov_rollup_interface::services::da::{DaService};
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_sequencer::get_sequencer_rpc;
use sov_stf_runner::{from_toml_path, get_ledger_rpc, StateTransitionRunner};
use crate::config::Config;
use sov_state::Storage;
use tracing::{debug, Level};

#[cfg(test)]
mod test_rpc;

pub fn initialize_ledger(path: impl AsRef<std::path::Path>) -> LedgerDB {
    LedgerDB::with_path(path).expect("Ledger DB failed to open")
}

// TODO: Remove this when sov-cli is in its own crate.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct HexKey {
    hex_priv_key: String,
    address: String,
}

pub fn get_genesis_config(sequencer_da_address: &str) -> GenesisConfig<DefaultContext> {
    let hex_key: HexKey = serde_json::from_slice(include_bytes!(
        "../../test-data/keys/token_deployer_private_key.json"
    ))
    .expect("Broken key data file");
    let sequencer_private_key = DefaultPrivateKey::from_hex(&hex_key.hex_priv_key).unwrap();
    assert_eq!(
        sequencer_private_key.default_address().to_string(),
        hex_key.address,
        "Inconsistent key data",
    );
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
    let config: Config =
        from_toml_path(&rollup_config_path).context("Failed to read rollup configuration")?;

    // Initializing logging
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    // Initialize the ledger database, which stores blocks, transactions, events, etc.
    let ledger_db = initialize_ledger(&config.rollup_config.runner.storage.path);

    let node_client = presence::build_client(config.da.node_client_url.to_string(), false)
        .await
        .unwrap();
    let light_client_url = config.da.light_client_url.to_string();
    // Initialize the Avail service using the DaService interface
    let da_service = AvailDaProvider {
        node_client,
        light_client_url,
    };

    let mut app = App::<Risc0Verifier, AvailBlobTransaction>::new(config.rollup_config.runner.storage.clone());

    let storage = app.get_storage();
    let mut methods = get_rpc_methods::<DefaultContext>(storage);

    // register rpc methods
    {
        register_ledger(ledger_db.clone(), &mut methods)?;
        register_sequencer(da_service.clone(), &mut app, &mut methods)?;
    }

    let storage = app.get_storage();
    let genesis_config = get_genesis_config(&config.sequencer_da_address);

    let mut runner = StateTransitionRunner::new(
        config.rollup_config,
        da_service,
        ledger_db,
        app.stf,
        storage.is_empty(),
        genesis_config,
    )?;

    runner.start_rpc_server(methods).await;
    runner.run().await?;

    Ok(())
}

fn register_sequencer<DA>(
    da_service: DA,
    demo_runner: &mut App<Risc0Verifier, AvailBlobTransaction>,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error>
where
    DA: DaService<Error = anyhow::Error> + Send + Sync + 'static,
{
    let batch_builder = demo_runner.batch_builder.take().unwrap();
    let sequencer_rpc = get_sequencer_rpc(batch_builder, Arc::new(da_service));
    methods
        .merge(sequencer_rpc)
        .context("Failed to merge Txs RPC modules")
}

fn register_ledger(
    ledger_db: LedgerDB,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error> {
    let ledger_rpc = get_ledger_rpc::<SequencerOutcome, TxEffect>(ledger_db);
    methods
        .merge(ledger_rpc)
        .context("Failed to merge ledger RPC modules")
}
