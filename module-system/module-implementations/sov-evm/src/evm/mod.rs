use reth_primitives::{Address, H256, U256};
use revm::primitives::specification::SpecId;
use revm::primitives::{ExecutionResult, Output, B160};
use serde::{Deserialize, Serialize};
use sov_state::{Prefix, StateMap};

pub(crate) mod conversions;
pub(crate) mod db;
mod db_commit;
pub(crate) mod db_init;
pub(crate) mod executor;
#[cfg(test)]
mod tests;
pub(crate) mod transaction;

pub use conversions::prepare_call_env;
use sov_state::codec::BcsCodec;
pub use transaction::RlpEvmTransaction;

// Stores information about an EVM account
#[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Default)]
pub(crate) struct AccountInfo {
    pub(crate) balance: U256,
    pub(crate) code_hash: H256,
    // TODO: `code` can be a huge chunk of data. We can use `StateValue` and lazy load it only when needed.
    // https://github.com/Sovereign-Labs/sovereign-sdk/issues/425
    pub(crate) code: Vec<u8>,
    pub(crate) nonce: u64,
}

/// Stores information about an EVM account and a corresponding account state.
#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
pub(crate) struct DbAccount {
    pub(crate) info: AccountInfo,
    pub(crate) storage: StateMap<U256, U256, BcsCodec>,
}

impl DbAccount {
    fn new(parent_prefix: &Prefix, address: Address) -> Self {
        let prefix = Self::create_storage_prefix(parent_prefix, address);
        Self {
            info: Default::default(),
            storage: StateMap::with_codec(prefix, BcsCodec {}),
        }
    }

    fn new_with_info(parent_prefix: &Prefix, address: Address, info: AccountInfo) -> Self {
        let prefix = Self::create_storage_prefix(parent_prefix, address);
        Self {
            info,
            storage: StateMap::with_codec(prefix, BcsCodec {}),
        }
    }

    fn create_storage_prefix(parent_prefix: &Prefix, address: Address) -> Prefix {
        let mut prefix = parent_prefix.as_aligned_vec().clone().into_inner();
        prefix.extend_from_slice(&address.0);
        Prefix::new(prefix)
    }
}

pub(crate) fn contract_address(result: ExecutionResult) -> Option<B160> {
    match result {
        ExecutionResult::Success {
            output: Output::Create(_, Some(addr)),
            ..
        } => Some(addr),
        _ => None,
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

    /// List of EVM hardforks by block number
    pub spec: Vec<(u64, SpecId)>,

    /// Coinbase where all the fees go
    pub coinbase: Address,

    /// Gas limit for single block
    pub block_gas_limit: u64,

    /// Delta to add to parent block timestamp
    pub block_timestamp_delta: u64,
}

impl Default for EvmChainConfig {
    fn default() -> EvmChainConfig {
        EvmChainConfig {
            chain_id: 1,
            limit_contract_code_size: None,
            spec: vec![(0, SpecId::LATEST)],
            coinbase: Address::zero(),
            block_gas_limit: reth_primitives::constants::ETHEREUM_BLOCK_GAS_LIMIT,
            block_timestamp_delta: 1,
        }
    }
}
