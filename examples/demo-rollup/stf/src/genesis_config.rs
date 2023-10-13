//! While the `GenesisConfig` type for `Rollup` is generated from the underlying runtime through a macro,
//! specific module configurations are obtained from files. This code is responsible for the logic
//! that transforms module genesis data into Rollup genesis data.

use std::convert::AsRef;
use std::path::{Path, PathBuf};

use anyhow::bail;
use sov_accounts::AccountConfig;
use sov_bank::BankConfig;
use sov_chain_state::ChainStateConfig;
#[cfg(feature = "experimental")]
use sov_evm::EvmConfig;
pub use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::Context;
use sov_nft_module::NonFungibleTokenConfig;
use sov_rollup_interface::da::DaSpec;
use sov_sequencer_registry::SequencerConfig;
pub use sov_state::config::Config as StorageConfig;
use sov_stf_runner::read_json_file;
use sov_value_setter::ValueSetterConfig;

/// Creates config for a rollup with some default settings, the config is used in demos and tests.
use crate::runtime::GenesisConfig;

/// Paths pointing to genesis files.
pub struct GenesisPaths<P: AsRef<Path>> {
    /// Bank genesis path.
    pub bank_genesis_path: P,
    /// Sequencer Registry genesis path.
    pub sequencer_genesis_path: P,
    /// Value Setter genesis path.
    pub value_setter_genesis_path: P,
    /// Accounts genesis path.
    pub accounts_genesis_path: P,
    /// Chain State genesis path.
    pub chain_state_genesis_path: P,
    /// NFT genesis path.
    pub nft_path: P,
    #[cfg(feature = "experimental")]
    /// EVM genesis path.
    pub evm_genesis_path: P,
}

impl GenesisPaths<PathBuf> {
    /// Creates a new [`GenesisPaths`] from the files contained in the given
    /// directory.
    ///
    /// Take a look at the contents of the `test_data` directory to see the
    /// expected files.
    pub fn from_dir(dir: impl AsRef<Path>) -> Self {
        Self {
            bank_genesis_path: dir.as_ref().join("bank.json"),
            sequencer_genesis_path: dir.as_ref().join("sequencer_registry.json"),
            value_setter_genesis_path: dir.as_ref().join("value_setter.json"),
            accounts_genesis_path: dir.as_ref().join("accounts.json"),
            chain_state_genesis_path: dir.as_ref().join("chain_state.json"),
            nft_path: dir.as_ref().join("nft.json"),
            #[cfg(feature = "experimental")]
            evm_genesis_path: dir.as_ref().join("evm.json"),
        }
    }
}

/// Configure our rollup with a centralized sequencer using the SEQUENCER_DA_ADDRESS
/// address constant. Since the centralize sequencer's address is consensus critical,
/// it has to be hardcoded as a constant, rather than read from the config at runtime.
///
/// If you want to customize the rollup to accept transactions from your own celestia
/// address, simply change the value of the SEQUENCER_DA_ADDRESS to your own address.
/// For example:
/// ```
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
        let token_address = &sov_bank::get_genesis_token_address::<C>(
            &bank_config.tokens[0].token_name,
            bank_config.tokens[0].salt,
        );

        let coins_token_addr = &sequencer_registry_config.coins_to_lock.token_address;
        if coins_token_addr != token_address {
            bail!(
                "Wrong token address in `sequencer_registry_config` expected {} but found {}",
                token_address,
                coins_token_addr
            )
        }
    }

    let value_setter_config: ValueSetterConfig<C> =
        read_json_file(&genesis_paths.value_setter_genesis_path)?;

    let accounts_config: AccountConfig<C> = read_json_file(&genesis_paths.accounts_genesis_path)?;
    let nft_config: NonFungibleTokenConfig = read_json_file(&genesis_paths.nft_path)?;

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
        nft_config,
        #[cfg(feature = "experimental")]
        evm_config,
    ))
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
