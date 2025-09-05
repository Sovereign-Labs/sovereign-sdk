use super::{DbAccount, EvmDb};
use alloy_primitives::{Address, Bytes, B256};
use revm::state::AccountInfo;
use sov_modules_api::StateReader;
use sov_modules_api::{Spec, StateAccessor};
use sov_state::User;

/// Initializes database with a predefined account.
pub(crate) trait InitEvmDb {
    type Error;

    fn insert_account_info(
        &mut self,
        address: Address,
        acc: AccountInfo,
    ) -> Result<(), Self::Error>;
    fn insert_code(&mut self, code_hash: B256, code: Bytes) -> Result<(), Self::Error>;
}

impl<'a, Ws: StateAccessor, S: Spec> InitEvmDb for EvmDb<'a, Ws, S> {
    type Error = <Ws as StateReader<User>>::Error;

    fn insert_account_info(
        &mut self,
        sender: Address,
        info: AccountInfo,
    ) -> Result<(), Self::Error> {
        let db_account = DbAccount(info);
        self.accounts.set(&sender, &db_account, self.state)
    }

    fn insert_code(&mut self, code_hash: B256, code: Bytes) -> Result<(), Self::Error> {
        self.code.set(&code_hash, &code, self.state)
    }
}
