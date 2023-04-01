use crate::Bank;
use anyhow::Result;
use sov_state::WorkingSet;

impl<C: sov_modules_api::Context> Bank<C> {
    pub(crate) fn init_module(&self, _working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        // TODO read initial tokens from "Config"
        Ok(())
    }
}
