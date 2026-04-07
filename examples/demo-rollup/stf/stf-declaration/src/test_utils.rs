//! Test utilities for the declaration crate's GenesisConfig.

use sov_address::{EthereumAddress, FromVmAddress};
use sov_evm::Evm;
use sov_hyperlane_integration::HyperlaneAddress;
use sov_modules_api::Base58Address;
use sov_modules_api::{Genesis, Spec};
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::Paymaster;
use sov_test_modules::access_pattern::AccessPattern;
use sov_test_utils::runtime::genesis::zk::MinimalZkGenesisConfig;

use crate::GenesisConfig;

impl<S: Spec> GenesisConfig<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
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
            paymaster: paymaster_config,
            revenue_share: (),
            mailbox: (),
            interchain_gas_paymaster: (),
            merkle_tree_hook: (),
            warp: (),
            evm: evm_config,
            synthetic_load: (),
            access_pattern: access_pattern_config,
        }
    }
}

impl<S: Spec> GenesisConfig<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    /// Creates a [`GenesisParams`] from a [`GenesisConfig`].
    pub fn into_genesis_params(self) -> GenesisParams<Self> {
        GenesisParams { runtime: self }
    }
}
