// Much of this code was copy-pasted from reth-evm, and we'd rather keep it as
// similar as possible to upstream than clean it up.
#![allow(clippy::match_same_arms)]

use alloy_eips::eip1559::BaseFeeParams;
use reth_primitives::revm_primitives::{AccountInfo, Address, SpecId};
use serde::{Deserialize, Serialize};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::config_value;
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

/// EVM Chain configuration
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EvmChainConfig {
    /// Unique chain id
    /// Chains can be registered at <https://github.com/ethereum-lists/chains>.
    pub chain_id: u64,

    /// Limits size of contract code size
    /// By default it is 0x6000 (~25kb).
    pub limit_contract_code_size: Option<usize>,

    /// List of EVM hard forks by block number
    pub spec: Vec<(u64, SpecId)>,

    /// Coinbase where all the fees go
    pub coinbase: Address,

    /// Gas limit for single block
    pub block_gas_limit: u64,

    /// Delta to add to parent block timestamp
    pub block_timestamp_delta: u64,

    /// Base fee params.
    pub base_fee_params: BaseFeeParams,
}

impl Default for EvmChainConfig {
    fn default() -> EvmChainConfig {
        EvmChainConfig {
            chain_id: config_value!("CHAIN_ID"),
            limit_contract_code_size: None,
            spec: vec![(0, SpecId::SHANGHAI)],
            coinbase: Address::ZERO,
            block_gas_limit: reth_primitives::constants::ETHEREUM_BLOCK_GAS_LIMIT,
            block_timestamp_delta: 1,
            base_fee_params: BaseFeeParams::ethereum(),
        }
    }
}

pub(crate) fn to_rollup_address<S: Spec>(address: reth_primitives::Address) -> S::Address
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    S::Address::from_vm_address(EthereumAddress::from(address))
}
