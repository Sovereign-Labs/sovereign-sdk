#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use std::str::FromStr;

use const_rollup_config::{ROLLUP_NAMESPACE_RAW, SEQUENCER_DA_ADDRESS};
use demo_stf::app::DefaultPrivateKey;
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runtime::GenesisConfig;
use jupiter::types::NamespaceId;
use jupiter::verifier::address::CelestiaAddress;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::default_context::DefaultContext;
pub mod register_rpc;

#[cfg(feature = "experimental")]
const TX_SIGNER_PRIV_KEY_PATH: &str = "../test-data/keys/tx_signer_private_key.json";

/// The rollup stores its data in the namespace b"sov-test" on Celestia
/// You can change this constant to point your rollup at a different namespace
pub const ROLLUP_NAMESPACE: NamespaceId = NamespaceId(ROLLUP_NAMESPACE_RAW);

/// Initializes a [`LedgerDB`] using the provided `path`.
pub fn initialize_ledger(path: impl AsRef<std::path::Path>) -> LedgerDB {
    LedgerDB::with_path(path).expect("Ledger DB failed to open")
}

/// TODO: Remove this when sov-cli is in its own crate.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct HexKey {
    hex_priv_key: String,
    address: String,
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
    let sequencer_da_address = CelestiaAddress::from_str(SEQUENCER_DA_ADDRESS).unwrap();
    create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        sequencer_da_address.as_ref().to_vec(),
        &sequencer_private_key,
        &sequencer_private_key,
    )
}
