#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

pub mod register_rpc;
mod rollup;

use const_rollup_config::ROLLUP_NAMESPACE_RAW;
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runtime::GenesisConfig;
pub use rollup::{
    new_rollup_with_celestia_da, new_rollup_with_mock_da, new_rollup_with_mock_da_from_config,
    Rollup,
};
use sov_celestia_adapter::types::NamespaceId;
use sov_cli::wallet_state::PrivateKeyAndAddress;
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
    #[cfg(feature = "experimental")] eth_accounts: Vec<reth_primitives::Address>,
) -> GenesisConfig<DefaultContext, Da> {
    let token_deployer_data =
        std::fs::read_to_string("../test-data/keys/token_deployer_private_key.json")
            .expect("Unable to read file to string");

    let token_deployer: PrivateKeyAndAddress<DefaultContext> =
        serde_json::from_str(&token_deployer_data).unwrap_or_else(|_| {
            panic!(
                "Unable to convert data {} to PrivateKeyAndAddress",
                &token_deployer_data
            )
        });

    assert!(
        token_deployer.is_matching_to_default(),
        "Inconsistent key data"
    );

    // TODO: #840
    create_demo_genesis_config(
        100000000,
        token_deployer.address,
        sequencer_da_address.as_ref().to_vec(),
        &token_deployer.private_key,
        #[cfg(feature = "experimental")]
        eth_accounts,
    )
}
