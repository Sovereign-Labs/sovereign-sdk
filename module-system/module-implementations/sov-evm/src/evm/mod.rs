use sov_state::Prefix;

mod conversions;
pub(crate) mod db;
mod db_commit;
pub(crate) mod db_init;
pub(crate) mod executor;
#[cfg(test)]
mod tests;

pub(crate) type Address = [u8; 20];
pub(crate) type SovU256 = [u8; 32];

// Stores information about an EVM account
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone, Default)]
pub(crate) struct AccountInfo {
    pub(crate) balance: SovU256,
    pub(crate) code_hash: SovU256,
    // TODO: `code` can be a huge chunk of data. We can use `StateValue` and lazy load it only when needed.
    // https://github.com/Sovereign-Labs/sovereign-sdk/issues/425
    pub(crate) code: Vec<u8>,
    pub(crate) nonce: u64,
}

/// Stores information about an EVM account and a corresponding account state.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub(crate) struct DbAccount {
    pub(crate) info: AccountInfo,
    pub(crate) storage: sov_state::StateMap<SovU256, SovU256>,
}

impl DbAccount {
    fn new(parent_prefix: &Prefix, address: Address) -> Self {
        let prefix = Self::create_storage_prefix(parent_prefix, address);
        Self {
            info: Default::default(),
            storage: sov_state::StateMap::new(prefix),
        }
    }

    fn new_with_info(parent_prefix: &Prefix, address: Address, info: AccountInfo) -> Self {
        let prefix = Self::create_storage_prefix(parent_prefix, address);
        Self {
            info,
            storage: sov_state::StateMap::new(prefix),
        }
    }

    fn create_storage_prefix(parent_prefix: &Prefix, address: Address) -> Prefix {
        let mut prefix = parent_prefix.as_aligned_vec().clone().into_inner();
        prefix.extend_from_slice(&address);
        Prefix::new(prefix)
    }
}
