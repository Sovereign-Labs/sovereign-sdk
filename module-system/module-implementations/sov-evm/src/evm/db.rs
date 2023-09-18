use std::convert::Infallible;

use reth_primitives::{Address, Bytes, H256};
use revm::primitives::{AccountInfo as ReVmAccountInfo, Bytecode, B160, B256, U256};
use revm::Database;
use sov_modules_api::WorkingSet;
use sov_state::codec::BcsCodec;

use super::DbAccount;

pub(crate) struct EvmDb<'a, C: sov_modules_api::Context> {
    pub(crate) accounts: sov_modules_api::StateMap<Address, DbAccount, BcsCodec>,
    pub(crate) code: sov_modules_api::StateMap<H256, Bytes, BcsCodec>,
    pub(crate) working_set: &'a mut WorkingSet<C>,
}

impl<'a, C: sov_modules_api::Context> EvmDb<'a, C> {
    pub(crate) fn new(
        accounts: sov_modules_api::StateMap<Address, DbAccount, BcsCodec>,
        code: sov_modules_api::StateMap<H256, Bytes, BcsCodec>,
        working_set: &'a mut WorkingSet<C>,
    ) -> Self {
        Self {
            accounts,
            code,
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

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        // TODO move to new_raw_with_hash for better performance
        let bytecode = Bytecode::new_raw(
            self.code
                .get(&code_hash, self.working_set)
                .unwrap_or(Bytes::default())
                .into(),
        );

        Ok(bytecode)
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
