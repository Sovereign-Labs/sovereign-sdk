use super::ValueSetter;
use super::ADMIN;
use anyhow::Result;

impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(&mut self) -> Result<()> {
        self.admin.set(ADMIN);
        Ok(())
    }
}
