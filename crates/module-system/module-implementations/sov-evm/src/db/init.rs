use alloy_primitives::{Address, Bytes, B256};
use revm::state::AccountInfo;
use sov_modules_api::{Spec, StateAccessor};

use super::{DbAccount, EvmDb};

/// Initializes database with a predefined account.
pub(crate) trait InitEvmDb {
    fn insert_account_info(&mut self, address: Address, acc: AccountInfo);
    fn insert_code(&mut self, code_hash: B256, code: Bytes);
}

impl<'a, Ws: StateAccessor, S: Spec> InitEvmDb for EvmDb<'a, Ws, S> {
    fn insert_account_info(&mut self, sender: Address, info: AccountInfo) {
        let db_account = DbAccount(info);

        self.accounts
            .set(&sender, &db_account, self.state)
            .expect("Failed to set account info");
    }

    fn insert_code(&mut self, code_hash: B256, code: Bytes) {
        self.code
            .set(&code_hash, &code, self.state)
            .expect("Failed to set account info");
    }
}

#[cfg(test)]
impl InitEvmDb for revm::database::CacheDB<revm::database::EmptyDB> {
    fn insert_account_info(&mut self, sender: Address, acc: AccountInfo) {
        self.insert_account_info(sender, acc);
    }

    fn insert_code(&mut self, code_hash: B256, code: Bytes) {
        use revm::state::Bytecode;

        self.cache
            .contracts
            .insert(code_hash, Bytecode::new_raw(code));
    }
}
