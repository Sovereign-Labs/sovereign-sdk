use std::convert::AsRef;
use std::path::Path;

use anyhow::Context as AnyhowContext;
use serde::de::DeserializeOwned;
use sov_accounts::AccountConfig;
use sov_bank::BankConfig;
use sov_chain_state::ChainStateConfig;
use sov_cli::wallet_state::PrivateKeyAndAddress;
#[cfg(feature = "experimental")]
use sov_evm::EvmConfig;
pub use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::Context;
use sov_nft_module::NonFungibleTokenConfig;
use sov_rollup_interface::da::DaSpec;
use sov_sequencer_registry::SequencerConfig;
pub use sov_state::config::Config as StorageConfig;
use sov_value_setter::ValueSetterConfig;

/// Creates config for a rollup with some default settings, the config is used in demos and tests.
use crate::runtime::GenesisConfig;

pub const LOCKED_AMOUNT: u64 = 50;
pub const DEMO_TOKEN_NAME: &str = "sov-demo-token";

/// Paths pointing to genesis files.
pub struct GenesisPaths<P: AsRef<Path>> {
    pub bank_genesis_path: P,
    pub sequencer_genesis_path: P,
    pub value_setter_genesis_path: P,
    pub accounts_genesis_path: P,
    pub chain_state_genesis_path: P,
    pub evm_genesis_path: P,
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
pub fn get_genesis_config<C: Context, Da: DaSpec, P: AsRef<Path>>(
    sequencer_da_address: Da::Address,
    genesis_paths: &GenesisPaths<P>,
    #[cfg(feature = "experimental")] eth_signers: Vec<reth_primitives::Address>,
) -> GenesisConfig<C, Da> {
    create_genesis_config(
        sequencer_da_address,
        genesis_paths,
        #[cfg(feature = "experimental")]
        eth_signers,
    )
    .expect("Unable to read genesis configuration")
}

fn create_genesis_config<C: Context, Da: DaSpec, P: AsRef<Path>>(
    seq_da_address: Da::Address,
    genesis_paths: &GenesisPaths<P>,
    #[cfg(feature = "experimental")] eth_signers: Vec<reth_primitives::Address>,
) -> anyhow::Result<GenesisConfig<C, Da>> {
    let bank_config: BankConfig<C> = read_json_file(&genesis_paths.bank_genesis_path)?;

    let mut sequencer_registry_config: SequencerConfig<C, Da> =
        read_json_file(&genesis_paths.sequencer_genesis_path)?;

    // The `seq_da_address` is overridden with the value from rollup binary.
    sequencer_registry_config.seq_da_address = seq_da_address;

    // Sanity check: `token_address` in `sequencer_registry_config` match `token_address` from the bank module.
    {
        let token_address = sov_bank::get_genesis_token_address::<C>(
            &bank_config.tokens[0].token_name,
            bank_config.tokens[0].salt,
        );

        assert_eq!(
            sequencer_registry_config.coins_to_lock.token_address,
            token_address
        );
    }

    let value_setter_config: ValueSetterConfig<C> =
        read_json_file(&genesis_paths.value_setter_genesis_path)?;

    let accounts_config: AccountConfig<C> = read_json_file(&genesis_paths.accounts_genesis_path)?;

    let nft_config: NonFungibleTokenConfig = NonFungibleTokenConfig {};

    let chain_state_config: ChainStateConfig =
        read_json_file(&genesis_paths.chain_state_genesis_path)?;

    #[cfg(feature = "experimental")]
    let evm_config = get_evm_config(&genesis_paths.evm_genesis_path, eth_signers)?;

    Ok(GenesisConfig::new(
        bank_config,
        sequencer_registry_config,
        (),
        chain_state_config,
        value_setter_config,
        accounts_config,
        #[cfg(feature = "experimental")]
        evm_config,
        nft_config,
    ))
}

fn read_json_file<T: DeserializeOwned, P: AsRef<Path>>(path: P) -> anyhow::Result<T> {
    let path_str = path.as_ref().display();

    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read genesis from {}", path_str))?;
    let config: T = serde_json::from_str(&data)
        .with_context(|| format!("Failed to parse genesis from {}", path_str))?;

    Ok(config)
}

#[cfg(feature = "experimental")]
fn get_evm_config<P: AsRef<Path>>(
    evm_path: P,
    signers: Vec<reth_primitives::Address>,
) -> anyhow::Result<EvmConfig> {
    let config: EvmConfig = read_json_file(evm_path)?;
    let addresses: std::collections::HashSet<reth_primitives::Address> =
        config.data.iter().map(|acc| acc.address).collect();

    // check if all the eth signer are in genesis.
    for signer in signers {
        assert!(addresses.contains(&signer));
    }

    Ok(config)
}

pub fn read_private_key<C: Context>() -> PrivateKeyAndAddress<C> {
    // TODO fix the hardcoded path: #872
    let token_deployer_data =
        std::fs::read_to_string("../test-data/keys/token_deployer_private_key.json")
            .expect("Unable to read file to string");

    let token_deployer: PrivateKeyAndAddress<C> = serde_json::from_str(&token_deployer_data)
        .unwrap_or_else(|_| {
            panic!(
                "Unable to convert data {} to PrivateKeyAndAddress",
                &token_deployer_data
            )
        });

    assert!(
        token_deployer.is_matching_to_default(),
        "Inconsistent key data"
    );

    token_deployer
}
