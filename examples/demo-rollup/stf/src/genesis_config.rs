//! While the `GenesisConfig` type for `Rollup` is generated from the underlying runtime through a macro,
//! specific module configurations are obtained from files. This code is responsible for the logic
//! that transforms module genesis data into Rollup genesis data.

use std::convert::AsRef;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
pub use sov_accounts::{AccountConfig, AccountData};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_attester_incentives::AttesterIncentivesConfig;
pub use sov_bank::{BankConfig, Coins, TokenConfig};
pub use sov_chain_state::ChainStateConfig;
pub use sov_evm::EvmConfig;
use sov_modules_api::prelude::*;
use sov_modules_stf_blueprint::Runtime as RuntimeTrait;
use sov_operator_incentives::OperatorIncentivesConfig;
use sov_paymaster::PaymasterConfig;
use sov_prover_incentives::ProverIncentivesConfig;
pub use sov_sequencer_registry::{SequencerConfig, SequencerRegistryConfig};
pub use sov_state::config::Config as StorageConfig;

/// Creates config for a rollup with some default settings, the config is used in demos and tests.
use crate::runtime::GenesisConfig;
use crate::runtime::Runtime;

/// Paths pointing to genesis files.
#[derive(Clone, Debug)]
pub struct GenesisPaths {
    /// Bank genesis path.
    pub bank_genesis_path: PathBuf,
    /// Sequencer Registry genesis path.
    pub sequencer_genesis_path: PathBuf,
    /// Accounts genesis path.
    pub accounts_genesis_path: PathBuf,
    /// Operator Incentives genesis path.
    pub operator_incentives_genesis_path: PathBuf,
    /// Attester Incentives genesis path.
    pub attester_incentives_genesis_path: PathBuf,
    /// Prover Incentives genesis path.
    pub prover_incentives_genesis_path: PathBuf,
    /// EVM genesis path.
    pub evm_genesis_path: PathBuf,
    /// Chain State genesis path.
    pub chain_state_genesis_path: PathBuf,
    /// Paymaster genesis path
    pub paymaster_genesis_path: PathBuf,
    /// Bench pattern genesis path
    pub access_pattern: PathBuf,
}

impl GenesisPaths {
    /// Creates a new [`GenesisPaths`] from the files contained in the given
    /// directory.
    ///
    /// Take a look at the contents of the `test-data` directory to see the
    /// expected files.
    pub fn from_dir(dir: impl AsRef<Path>) -> Self {
        Self {
            bank_genesis_path: dir.as_ref().join("bank.json"),
            sequencer_genesis_path: dir.as_ref().join("sequencer_registry.json"),
            accounts_genesis_path: dir.as_ref().join("accounts.json"),
            operator_incentives_genesis_path: dir.as_ref().join("operator_incentives.json"),
            prover_incentives_genesis_path: dir.as_ref().join("prover_incentives.json"),
            attester_incentives_genesis_path: dir.as_ref().join("attester_incentives.json"),
            evm_genesis_path: dir.as_ref().join("evm.json"),
            chain_state_genesis_path: dir.as_ref().join("chain_state.json"),
            paymaster_genesis_path: dir.as_ref().join("paymaster.json"),
            access_pattern: dir.as_ref().join("access_pattern.json"),
        }
    }
}

/// Creates a new [`RuntimeTrait::GenesisConfig`] from the files contained in
/// the given directory.
pub fn create_genesis_config<S: Spec>(
    genesis_paths: &GenesisPaths,
) -> anyhow::Result<<Runtime<S> as RuntimeTrait<S>>::GenesisConfig>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    let bank_config: BankConfig<S> = read_genesis_json(&genesis_paths.bank_genesis_path)?;

    let sequencer_registry_config: SequencerRegistryConfig<S> =
        read_genesis_json(&genesis_paths.sequencer_genesis_path)?;

    let operator_incentives_config: OperatorIncentivesConfig<S> =
        read_genesis_json(&genesis_paths.operator_incentives_genesis_path)?;

    let attester_incentives_config: AttesterIncentivesConfig<S> =
        read_genesis_json(&genesis_paths.attester_incentives_genesis_path)?;

    let prover_incentives_config: ProverIncentivesConfig<S> =
        read_genesis_json(&genesis_paths.prover_incentives_genesis_path)?;

    let accounts_config: AccountConfig<S> =
        read_genesis_json(&genesis_paths.accounts_genesis_path)?;

    let nonces_config = ();

    let evm_config: EvmConfig = read_genesis_json(&genesis_paths.evm_genesis_path)?;

    let chain_state_config: ChainStateConfig<S> =
        read_genesis_json(&genesis_paths.chain_state_genesis_path)?;
    let paymaster_config: PaymasterConfig<S> =
        read_genesis_json(&genesis_paths.paymaster_genesis_path)?;
    let blob_storage_config = ();

    let access_pattern: sov_test_modules::access_pattern::AccessPatternGenesisConfig<S> =
        read_genesis_json(&genesis_paths.access_pattern)?;

    let synthetic_load_config = ();

    Ok(GenesisConfig::new(
        bank_config,
        sequencer_registry_config,
        operator_incentives_config,
        attester_incentives_config,
        prover_incentives_config,
        accounts_config,
        nonces_config,
        chain_state_config,
        blob_storage_config,
        paymaster_config,
        evm_config,
        access_pattern,
        synthetic_load_config,
    ))
}

fn read_genesis_json<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let contents = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&contents)?)
}
