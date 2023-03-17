use crate::Accounts;
use anyhow::Result;

impl<C: sov_modules_api::Context> Accounts<C> {
    pub(crate) fn init_module(&mut self) -> Result<()> {
        // TODO read initial accounts from "Config"
        Ok(())
    }
}
