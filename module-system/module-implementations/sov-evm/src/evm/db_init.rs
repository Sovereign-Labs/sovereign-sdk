#[cfg(test)]
use revm::db::{CacheDB, EmptyDB};
use revm::primitives::Address;

use super::db::EvmDb;
use super::{AccountInfo, DbAccount};

/// Initializes database with a predefined account.
pub(crate) trait InitEvmDb {
    fn insert_account_info(&mut self, address: Address, acc: AccountInfo);
}

impl<'a, C: sov_modules_api::Context> InitEvmDb for EvmDb<'a, C> {
    fn insert_account_info(&mut self, sender: Address, info: AccountInfo) {
        let parent_prefix = self.accounts.prefix();
        let db_account = DbAccount::new_with_info(parent_prefix, sender, info);

        self.accounts.set(&sender, &db_account, self.working_set);
    }
}

#[cfg(test)]
impl InitEvmDb for CacheDB<EmptyDB> {
    fn insert_account_info(&mut self, sender: Address, acc: AccountInfo) {
        self.insert_account_info(sender, acc.into());
    }
}
