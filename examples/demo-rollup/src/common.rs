#[cfg(feature = "experimental")]
use std::str::FromStr;

use anyhow::Context as _;
use demo_stf::runtime::Runtime;
#[cfg(feature = "experimental")]
use secp256k1::SecretKey;
#[cfg(feature = "experimental")]
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_db::ledger_db::LedgerDB;
#[cfg(feature = "experimental")]
use sov_ethereum::experimental::EthRpcConfig;
#[cfg(feature = "experimental")]
use sov_ethereum::GasPriceOracleConfig;
use sov_modules_api::default_context::DefaultContext;
#[cfg(feature = "experimental")]
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::Spec;
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::services::batch_builder::BatchBuilder;
use sov_rollup_interface::services::da::DaService;
use sov_sequencer::batch_builder::FiFoStrictBatchBuilder;
use sov_sequencer::get_sequencer_rpc;
use sov_state::ProverStorage;

#[cfg(feature = "experimental")]
const TX_SIGNER_PRIV_KEY_PATH: &str = "../test-data/keys/tx_signer_private_key.json";

pub(crate) fn create_rpc_methods<Da: DaService + Clone>(
    storage: &<DefaultContext as Spec>::Storage,
    ledger_db: &LedgerDB,
    da_service: Da,
) -> Result<jsonrpsee::RpcModule<()>, anyhow::Error> {
    let batch_builder = create_batch_builder::<<Da as DaService>::Spec>(storage.clone());

    let mut methods = demo_stf::runtime::get_rpc_methods::<DefaultContext, <Da as DaService>::Spec>(
        storage.clone(),
    );

    methods.merge(
        sov_ledger_rpc::server::rpc_module::<
            LedgerDB,
            SequencerOutcome<<<Da as DaService>::Spec as DaSpec>::Address>,
            TxEffect,
        >(ledger_db.clone())?
        .remove_context(),
    )?;

    register_sequencer(da_service.clone(), batch_builder, &mut methods)?;

    #[cfg(feature = "experimental")]
    register_ethereum::<Da>(da_service.clone(), storage.clone(), &mut methods).unwrap();

    Ok(methods)
}

fn register_sequencer<Da: DaService, B: BatchBuilder + Send + Sync + 'static>(
    da_service: Da,
    batch_builder: B,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error> {
    let sequencer_rpc = get_sequencer_rpc(batch_builder, da_service);
    methods
        .merge(sequencer_rpc)
        .context("Failed to merge Txs RPC modules")
}

fn create_batch_builder<Da: DaSpec>(
    storage: ProverStorage<sov_state::DefaultStorageSpec>,
) -> FiFoStrictBatchBuilder<DefaultContext, Runtime<DefaultContext, Da>> {
    let batch_size_bytes = 1024 * 100; // 100 KB
    FiFoStrictBatchBuilder::new(
        batch_size_bytes,
        u32::MAX as usize,
        Runtime::default(),
        storage,
    )
}

#[cfg(feature = "experimental")]
/// Ethereum RPC wraps EVM transaction in a rollup transaction.
/// This function reads the private key of the rollup transaction signer.
fn read_sov_tx_signer_priv_key() -> Result<DefaultPrivateKey, anyhow::Error> {
    let data = std::fs::read_to_string(TX_SIGNER_PRIV_KEY_PATH).context("Unable to read file")?;

    let key_and_address: PrivateKeyAndAddress<DefaultContext> = serde_json::from_str(&data)
        .unwrap_or_else(|_| panic!("Unable to convert data {} to PrivateKeyAndAddress", &data));

    Ok(key_and_address.private_key)
}

// TODO: #840
#[cfg(feature = "experimental")]
pub(crate) fn read_eth_tx_signers() -> sov_ethereum::DevSigner {
    sov_ethereum::DevSigner::new(vec![SecretKey::from_str(
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    )
    .unwrap()])
}

#[cfg(feature = "experimental")]
// register ethereum methods.
fn register_ethereum<Da: DaService>(
    da_service: Da,
    storage: ProverStorage<sov_state::DefaultStorageSpec>,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error> {
    let eth_rpc_config = {
        let eth_signer = read_eth_tx_signers();
        EthRpcConfig::<DefaultContext> {
            min_blob_size: Some(1),
            sov_tx_signer_priv_key: read_sov_tx_signer_priv_key()?,
            eth_signer,
            gas_price_oracle_config: GasPriceOracleConfig::default(),
        }
    };

    let ethereum_rpc =
        sov_ethereum::get_ethereum_rpc::<DefaultContext, Da>(da_service, eth_rpc_config, storage);
    methods
        .merge(ethereum_rpc)
        .context("Failed to merge Ethereum RPC modules")
}
