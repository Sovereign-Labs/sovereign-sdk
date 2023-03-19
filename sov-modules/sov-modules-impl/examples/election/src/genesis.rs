use super::Election;
use anyhow::{anyhow, Result};
use sov_modules_api::PublicKey;

impl<C: sov_modules_api::Context> Election<C> {
    pub(crate) fn init_module(&mut self) -> Result<()> {
        let admin_pub_key = C::PublicKey::try_from("election_admin")
            .map_err(|_| anyhow!("Admin initialization failed"))?;

        self.admin.set(admin_pub_key.to_address());
        self.is_frozen.set(false);
        Ok(())
    }
}
