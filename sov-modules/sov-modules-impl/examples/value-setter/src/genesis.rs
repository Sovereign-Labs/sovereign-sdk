use super::ValueSetter;
use anyhow::Result;
use sov_modules_api::Address;

impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(&mut self) -> Result<()> {
        let admin: Address = Address::from("admin");
        self.admin.set(admin);
        Ok(())
    }
}
