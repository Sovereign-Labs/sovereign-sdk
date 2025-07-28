use reth_primitives::revm_primitives::{AccountInfo, Address, B256};
use reth_primitives::Bytes;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{InfallibleStateAccessor, Spec};

use super::db::EvmDb;
use super::DbAccount;

/// Initializes database with a predefined account.
pub(crate) trait InitEvmDb {
    fn insert_account_info(&mut self, address: Address, acc: AccountInfo);
    fn insert_code(&mut self, code_hash: B256, code: Bytes);
}

impl<Ws: InfallibleStateAccessor, S: Spec> InitEvmDb for EvmDb<Ws, S> {
    fn insert_account_info(&mut self, sender: Address, info: AccountInfo) {
        let db_account = DbAccount { info };

        self.accounts
            .set(&sender, &db_account, &mut self.state)
            .unwrap_infallible();
    }

    fn insert_code(&mut self, code_hash: B256, code: Bytes) {
        self.code
            .set(&code_hash, &code, &mut self.state)
            .unwrap_infallible();
    }
}

#[cfg(test)]
impl InitEvmDb for revm::db::CacheDB<revm::db::EmptyDB> {
    fn insert_account_info(&mut self, sender: Address, acc: AccountInfo) {
        self.insert_account_info(sender, acc);
    }

    fn insert_code(&mut self, code_hash: B256, code: Bytes) {
        self.contracts.insert(
            code_hash,
            reth_primitives::revm_primitives::Bytecode::new_raw(code),
        );
    }
}
