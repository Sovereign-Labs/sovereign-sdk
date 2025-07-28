//! This file implements traits only useful for testing.
//! It allows for compatibility of the [`Runtime`] with Sovereign's testing framework.
//! Users that want to fully use the testing framework with a custom runtime should implement these traits.
//! Note though that, in practice, the runtime trait (and the methods below) can be macro-derived using the
//! framework's macro exports. See `sov-test-utils` crate for additional information

use sov_address::{EthereumAddress, FromVmAddress};
use sov_evm::Evm;
use sov_modules_api::{Genesis, Spec};
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::Paymaster;
use sov_sequencer_registry::SequencerRegistry;
use sov_test_modules::access_pattern::AccessPattern;
use sov_test_utils::runtime::genesis::zk::MinimalZkGenesisConfig;
use sov_test_utils::runtime::traits::MinimalGenesis;

use crate::runtime::{GenesisConfig, Runtime};

impl<S: Spec> MinimalGenesis<S> for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Returns a reference to the sequencer registry config.
    fn sequencer_registry_config(
        config: &Self::Config,
    ) -> &<SequencerRegistry<S> as Genesis>::Config {
        &config.sequencer_registry
    }
}

impl<S: Spec> GenesisConfig<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Creates a new [`GenesisConfig`] from a minimal genesis config [`::sov_modules_api::Genesis::Config`].
    pub fn from_minimal_config(
        minimal_config: MinimalZkGenesisConfig<S>,
        evm_config: <Evm<S> as Genesis>::Config,
        paymaster_config: <Paymaster<S> as Genesis>::Config,
        access_pattern_config: <AccessPattern<S> as Genesis>::Config,
    ) -> Self {
        Self {
            sequencer_registry: minimal_config.config.sequencer_registry,
            bank: minimal_config.config.bank,
            accounts: minimal_config.config.accounts,
            uniqueness: minimal_config.config.uniqueness,
            chain_state: minimal_config.config.chain_state,
            blob_storage: minimal_config.config.blob_storage,
            operator_incentives: minimal_config.config.operator_incentives,
            prover_incentives: minimal_config.config.prover_incentives,
            attester_incentives: minimal_config.config.attester_incentives,
            evm: evm_config,
            paymaster: paymaster_config,
            synthetic_load: (),
            access_pattern: access_pattern_config,
        }
    }
}

impl<S: Spec> GenesisConfig<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Creates a [`$crate::runtime::GenesisParams`] from a [`GenesisConfig`].
    pub fn into_genesis_params(self) -> GenesisParams<Self> {
        GenesisParams { runtime: self }
    }
}
