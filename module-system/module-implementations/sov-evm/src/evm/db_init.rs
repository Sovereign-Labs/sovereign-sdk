use reth_primitives::Bytes;
#[cfg(test)]
use revm::db::{CacheDB, EmptyDB};
use revm::primitives::{Address, B256};

use super::db::EvmDb;
use super::{AccountInfo, DbAccount};

/// Initializes database with a predefined account.
pub(crate) trait InitEvmDb {
    fn insert_account_info(&mut self, address: Address, acc: AccountInfo);
    fn insert_code(&mut self, code_hash: B256, code: Bytes);
}

impl<'a, C: sov_modules_api::Context> InitEvmDb for EvmDb<'a, C> {
    fn insert_account_info(&mut self, sender: Address, info: AccountInfo) {
        let parent_prefix = self.accounts.prefix();
        let db_account = DbAccount::new_with_info(parent_prefix, sender, info);

        self.accounts.set(&sender, &db_account, self.working_set);
    }

    fn insert_code(&mut self, code_hash: B256, code: Bytes) {
        self.code.set(&code_hash, &code, self.working_set)
    }
}

#[cfg(test)]
impl InitEvmDb for CacheDB<EmptyDB> {
    fn insert_account_info(&mut self, sender: Address, acc: AccountInfo) {
        self.insert_account_info(sender, acc.into());
    }

    fn insert_code(&mut self, code_hash: B256, code: Bytes) {
        self.contracts
            .insert(code_hash, revm::primitives::Bytecode::new_raw(code.into()));
    }
}
