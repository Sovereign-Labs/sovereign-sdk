#[cfg(test)]
use revm::{
    db::{CacheDB, EmptyDB},
    primitives::B160,
};

use super::db::EvmDb;
use super::{AccountInfo, DbAccount, EthAddress};

/// Initializes database with a predefined account.
pub(crate) trait InitEvmDb {
    fn insert_account_info(&mut self, address: EthAddress, acc: AccountInfo);
}

impl<'a, C: sov_modules_api::Context> InitEvmDb for EvmDb<'a, C> {
    fn insert_account_info(&mut self, sender: EthAddress, info: AccountInfo) {
        let parent_prefix = self.accounts.prefix();
        let db_account = DbAccount::new_with_info(parent_prefix, sender, info);

        self.accounts.set(&sender, &db_account, self.working_set);
    }
}

#[cfg(test)]
impl InitEvmDb for CacheDB<EmptyDB> {
    fn insert_account_info(&mut self, sender: EthAddress, acc: AccountInfo) {
        self.insert_account_info(B160::from_slice(&sender), acc.into());
    }
}
