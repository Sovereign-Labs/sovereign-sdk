use revm::primitives::{Account, HashMap, B160};
use revm::DatabaseCommit;

use super::db::EvmDb;
use super::DbAccount;

impl<'a, C: sov_modules_api::Context> DatabaseCommit for EvmDb<'a, C> {
    fn commit(&mut self, changes: HashMap<B160, Account>) {
        for (address, account) in changes {
            let address = address;

            // TODO figure out what to do when account is destroyed.
            // https://github.com/Sovereign-Labs/sovereign-sdk/issues/425
            if account.is_destroyed {
                todo!("Account destruction not supported")
            }

            let accounts_prefix = self.accounts.prefix();

            let mut db_account = self
                .accounts
                .get(&address, self.working_set)
                .unwrap_or_else(|| DbAccount::new(accounts_prefix, address));

            db_account.info = account.info.into();

            for (key, value) in account.storage.into_iter() {
                let value = value.present_value();
                db_account.storage.set(&key, &value, self.working_set);
            }

            self.accounts.set(&address, &db_account, self.working_set)
        }
    }
}
