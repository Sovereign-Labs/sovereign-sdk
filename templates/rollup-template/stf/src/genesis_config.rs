use std::path::{Path, PathBuf};

use anyhow::bail;
use sov_accounts::AccountConfig;
use sov_bank::BankConfig;
use sov_modules_api::{Context, DaSpec};
use sov_sequencer_registry::SequencerConfig;
use sov_stf_runner::read_json_file;

use super::GenesisConfig;

/// Paths to genesis files.
pub struct GenesisPaths<P: AsRef<Path>> {
    /// Accounts genesis path.
    pub accounts_genesis_path: P,
    /// Bank genesis path.
    pub bank_genesis_path: P,
    /// Sequencer Registry genesis path.
    pub sequencer_genesis_path: P,
}

impl GenesisPaths<PathBuf> {
    /// Creates a new [`GenesisPaths`] from the files contained in the given
    /// directory.
    ///
    /// Take a look at the contents of the `test_data` directory to see the
    /// expected files.
    pub fn from_dir(dir: impl AsRef<Path>) -> Self {
        Self {
            accounts_genesis_path: dir.as_ref().join("accounts.json"),
            bank_genesis_path: dir.as_ref().join("bank.json"),
            sequencer_genesis_path: dir.as_ref().join("sequencer_registry.json"),
        }
    }
}

// Configure our rollup with a centralized sequencer using the SEQUENCER_DA_ADDRESS
/// address constant. Since the centralize sequencer's address is consensus critical,
/// it has to be hardcoded as a constant, rather than read from the config at runtime.
pub fn get_genesis_config<C: Context, Da: DaSpec, P: AsRef<Path>>(
    sequencer_da_address: Da::Address,
    genesis_paths: &GenesisPaths<P>,
) -> GenesisConfig<C, Da> {
    create_genesis_config(sequencer_da_address, genesis_paths)
        .expect("Unable to read genesis configuration")
}

fn create_genesis_config<C: Context, Da: DaSpec, P: AsRef<Path>>(
    seq_da_address: Da::Address,
    genesis_paths: &GenesisPaths<P>,
) -> anyhow::Result<GenesisConfig<C, Da>> {
    let accounts_config: AccountConfig<C> = read_json_file(&genesis_paths.accounts_genesis_path)?;
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

    Ok(GenesisConfig::new(
        accounts_config,
        bank_config,
        sequencer_registry_config,
    ))
}
