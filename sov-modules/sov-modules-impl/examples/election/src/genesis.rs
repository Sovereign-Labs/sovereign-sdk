use super::{Election, ADMIN};
use anyhow::Result;

impl<C: sov_modules_api::Context> Election<C> {
    pub(crate) fn init_module(&mut self) -> Result<()> {
        self.admin.set(ADMIN);
        self.is_frozen.set(false);
        Ok(())
    }
}
