use std::convert::Infallible;

use reth_primitives::Address;
use revm::primitives::{AccountInfo as ReVmAccountInfo, Bytecode, B160, B256, U256};
use revm::Database;
use sov_state::codec::BcsCodec;
use sov_state::WorkingSet;

use super::DbAccount;

pub(crate) struct EvmDb<'a, C: sov_modules_api::Context> {
    pub(crate) accounts: sov_state::StateMap<Address, DbAccount, BcsCodec>,
    pub(crate) working_set: &'a mut WorkingSet<C::Storage>,
}

impl<'a, C: sov_modules_api::Context> EvmDb<'a, C> {
    pub(crate) fn new(
        accounts: sov_state::StateMap<Address, DbAccount, BcsCodec>,
        working_set: &'a mut WorkingSet<C::Storage>,
    ) -> Self {
        Self {
            accounts,
            working_set,
        }
    }
}

impl<'a, C: sov_modules_api::Context> Database for EvmDb<'a, C> {
    type Error = Infallible;

    fn basic(&mut self, address: B160) -> Result<Option<ReVmAccountInfo>, Self::Error> {
        let db_account = self.accounts.get(&address, self.working_set);
        Ok(db_account.map(|acc| acc.info.into()))
    }

    fn code_by_hash(&mut self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        panic!("Should not be called. Contract code is already loaded");
    }

    fn storage(&mut self, address: B160, index: U256) -> Result<U256, Self::Error> {
        let storage_value: U256 = if let Some(acc) = self.accounts.get(&address, self.working_set) {
            acc.storage
                .get(&index, self.working_set)
                .unwrap_or_default()
        } else {
            U256::default()
        };

        Ok(storage_value)
    }

    fn block_hash(&mut self, _number: U256) -> Result<B256, Self::Error> {
        todo!("block_hash not yet implemented")
    }
}
