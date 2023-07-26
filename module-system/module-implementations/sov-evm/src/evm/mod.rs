use revm::primitives::{ExecutionResult, Output, B160};
use sov_state::Prefix;

mod conversions;
pub(crate) mod db;
mod db_commit;
pub(crate) mod db_init;
pub(crate) mod executor;
#[cfg(test)]
pub(crate) mod test_helpers;
#[cfg(test)]
mod tests;
pub(crate) mod transaction;

pub type EthAddress = [u8; 20];
pub(crate) type Bytes32 = [u8; 32];

pub use transaction::EvmTransaction;
// Stores information about an EVM account
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone, Default)]
pub(crate) struct AccountInfo {
    pub(crate) balance: Bytes32,
    pub(crate) code_hash: Bytes32,
    // TODO: `code` can be a huge chunk of data. We can use `StateValue` and lazy load it only when needed.
    // https://github.com/Sovereign-Labs/sovereign-sdk/issues/425
    pub(crate) code: Vec<u8>,
    pub(crate) nonce: u64,
}

/// Stores information about an EVM account and a corresponding account state.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub(crate) struct DbAccount {
    pub(crate) info: AccountInfo,
    pub(crate) storage: sov_state::StateMap<Bytes32, Bytes32>,
}

impl DbAccount {
    fn new(parent_prefix: &Prefix, address: EthAddress) -> Self {
        let prefix = Self::create_storage_prefix(parent_prefix, address);
        Self {
            info: Default::default(),
            storage: sov_state::StateMap::new(prefix),
        }
    }

    fn new_with_info(parent_prefix: &Prefix, address: EthAddress, info: AccountInfo) -> Self {
        let prefix = Self::create_storage_prefix(parent_prefix, address);
        Self {
            info,
            storage: sov_state::StateMap::new(prefix),
        }
    }

    fn create_storage_prefix(parent_prefix: &Prefix, address: EthAddress) -> Prefix {
        let mut prefix = parent_prefix.as_aligned_vec().clone().into_inner();
        prefix.extend_from_slice(&address);
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
