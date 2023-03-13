use super::Election;
use anyhow::{anyhow, Result};

impl<C: sov_modules_api::Context> Election<C> {
    pub(crate) fn init_module(&mut self) -> Result<()> {
        let admin =
            C::PublicKey::try_from("admin").map_err(|_| anyhow!("Admin initialization failed"))?;

        self.admin.set(admin);
        self.is_frozen.set(false);
        Ok(())
    }
}
