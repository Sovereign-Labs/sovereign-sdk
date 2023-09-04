#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

pub mod register_rpc;
mod rollup;

use celestia::types::NamespaceId;
use const_rollup_config::ROLLUP_NAMESPACE_RAW;
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runtime::GenesisConfig;
#[cfg(feature = "experimental")]
pub use rollup::read_tx_signer_priv_key;
pub use rollup::{
    new_rollup_with_celestia_da, new_rollup_with_mock_da, new_rollup_with_mock_da_from_config,
    Rollup,
};
use sov_cli::wallet_state::{HexPrivateAndAddress, PrivateKeyAndAddress};
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::default_context::DefaultContext;
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};

/// The rollup stores its data in the namespace b"sov-test" on Celestia
/// You can change this constant to point your rollup at a different namespace
pub const ROLLUP_NAMESPACE: NamespaceId = NamespaceId(ROLLUP_NAMESPACE_RAW);

/// Initializes a [`LedgerDB`] using the provided `path`.
pub fn initialize_ledger(path: impl AsRef<std::path::Path>) -> LedgerDB {
    LedgerDB::with_path(path).expect("Ledger DB failed to open")
}

/// Configure our rollup with a centralized sequencer using the SEQUENCER_DA_ADDRESS
/// address constant. Since the centralize sequencer's address is consensus critical,
/// it has to be hardcoded as a constant, rather than read from the config at runtime.
///
/// If you want to customize the rollup to accept transactions from your own celestia
/// address, simply change the value of the SEQUENCER_DA_ADDRESS to your own address.
/// For example:
/// ```rust,no_run
/// const SEQUENCER_DA_ADDRESS: &str = "celestia1qp09ysygcx6npted5yc0au6k9lner05yvs9208";
/// ```
pub fn get_genesis_config<Da: DaSpec>(
    sequencer_da_address: <<Da as DaSpec>::BlobTransaction as BlobReaderTrait>::Address,
) -> GenesisConfig<DefaultContext, Da> {
    let hex_key: HexPrivateAndAddress = serde_json::from_slice(include_bytes!(
        "../../test-data/keys/token_deployer_private_key.json"
    ))
    .expect("Broken key data file");
    let key_and_address: PrivateKeyAndAddress<DefaultContext> = hex_key
        .try_into()
        .expect("Failed to parse sequencer private key and address");
    assert!(
        key_and_address.is_matching_to_default(),
        "Inconsistent key data"
    );

    create_demo_genesis_config(
        100000000,
        key_and_address.address,
        sequencer_da_address.as_ref().to_vec(),
        &key_and_address.private_key,
    )
}
