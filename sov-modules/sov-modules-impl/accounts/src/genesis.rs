use crate::Accounts;
use anyhow::Result;
use sov_state::WorkingSet;

impl<C: sov_modules_api::Context> Accounts<C> {
    pub(crate) fn init_module(&mut self, _working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        // TODO read initial accounts from "Config"
        Ok(())
    }
}
