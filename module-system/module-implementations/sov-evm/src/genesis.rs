use anyhow::Result;
use sov_state::WorkingSet;

use crate::evm::db_init::InitEvmDb;
use crate::evm::AccountInfo;
use crate::Evm;

impl<C: sov_modules_api::Context> Evm<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let mut evm_db = self.get_db(working_set);

        for acc in &config.data {
            evm_db.insert_account_info(
                acc.address,
                AccountInfo {
                    balance: acc.balance,
                    code_hash: acc.code_hash,
                    code: acc.code.clone(),
                    nonce: acc.nonce,
                },
            )
        }
        Ok(())
    }
}
