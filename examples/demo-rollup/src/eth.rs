use std::str::FromStr;

use anyhow::Context as _;
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_ethereum::experimental::EthRpcConfig;
use sov_ethereum::GasPriceOracleConfig;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_prover_storage_manager::SnapshotManager;
use sov_rollup_interface::services::da::DaService;
use sov_state::ProverStorage;

const TX_SIGNER_PRIV_KEY_PATH: &str = "../test-data/keys/tx_signer_private_key.json";

/// Ethereum RPC wraps EVM transaction in a rollup transaction.
/// This function reads the private key of the rollup transaction signer.
fn read_sov_tx_signer_priv_key() -> Result<DefaultPrivateKey, anyhow::Error> {
    let data = std::fs::read_to_string(TX_SIGNER_PRIV_KEY_PATH).context("Unable to read file")?;

    let key_and_address: PrivateKeyAndAddress<DefaultContext> = serde_json::from_str(&data)
        .unwrap_or_else(|_| panic!("Unable to convert data {} to PrivateKeyAndAddress", &data));

    Ok(key_and_address.private_key)
}

// register ethereum methods.
pub(crate) fn register_ethereum<Da: DaService>(
    da_service: Da,
    storage: ProverStorage<sov_state::DefaultStorageSpec, SnapshotManager>,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error> {
    let eth_rpc_config = {
        let eth_signer = eth_dev_signer();
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

// TODO: #840
fn eth_dev_signer() -> sov_ethereum::DevSigner {
    sov_ethereum::DevSigner::new(vec![secp256k1::SecretKey::from_str(
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    )
    .unwrap()])
}
