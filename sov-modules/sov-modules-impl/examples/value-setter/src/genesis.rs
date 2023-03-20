use super::ValueSetter;
use anyhow::{anyhow, Result};
use sov_modules_api::PublicKey;

impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(&mut self) -> Result<()> {
        let admin_pub_key = C::PublicKey::try_from("value_setter_admin")
            .map_err(|_| anyhow!("Admin initialization failed"))?;

        self.admin.set(admin_pub_key.to_address());
        Ok(())
    }
}
