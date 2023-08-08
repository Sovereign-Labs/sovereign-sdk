use anyhow::Result;
use revm::primitives::SpecId;
use sov_state::WorkingSet;

use crate::evm::db_init::InitEvmDb;
use crate::evm::{AccountInfo, SpecIdWrapper};
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

        let mut spec = config
            .spec
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect::<Vec<_>>();

        spec.sort_by(|a, b| a.0.cmp(&b.0));

        if spec.is_empty() {
            spec.push((0, SpecIdWrapper::from(SpecId::LATEST)));
        } else if spec[0].0 != 0 {
            panic!("EVM spec must start from block 0");
        }

        self.spec.set(&spec, working_set);

        Ok(())
    }
}
