// Much of this code was copy-pasted from reth-evm, and we'd rather keep it as
// similar as possible to upstream than clean it up.
#![allow(clippy::match_same_arms)]

use alloy_primitives::Address;
use revm::primitives::hardfork::SpecId;
use revm::state::AccountInfo;
use serde::{Deserialize, Serialize};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::Spec;

pub(crate) mod conversions;
pub(crate) mod db;
mod db_commit;
pub(crate) mod db_init;
/// EVM execution utilities
pub mod executor;
pub(crate) mod primitive_types;

pub use primitive_types::RlpEvmTransaction;

/// Stores information about an EVM account and a corresponding account state.
#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
pub struct DbAccount {
    pub(crate) info: AccountInfo,
}

impl DbAccount {
    fn new() -> Self {
        Self {
            info: Default::default(),
        }
    }

    /// The account info associated with this db account.
    pub fn account_info(&self) -> &AccountInfo {
        &self.info
    }
}

/// Runtime configuration for EVM execution
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EvmRuntimeConfig {
    /// Core chain parameters
    pub chain_spec: crate::EvmChainSpec,
    /// Sorted hard fork schedule for efficient runtime lookup
    /// (block number, fork ID) ordered by block number
    pub hardforks: Vec<(u64, SpecId)>,
}

impl Default for EvmRuntimeConfig {
    fn default() -> EvmRuntimeConfig {
        let chain_spec = crate::EvmChainSpec::default();
        // Clone hardforks from chain_spec for runtime use
        let hardforks = chain_spec.hardforks.clone();

        EvmRuntimeConfig {
            chain_spec,
            hardforks,
        }
    }
}

pub(crate) fn to_rollup_address<S: Spec>(address: Address) -> S::Address
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    S::Address::from_vm_address(EthereumAddress::from(address))
}
