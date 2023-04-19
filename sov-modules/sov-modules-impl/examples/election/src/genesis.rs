use super::Election;
use anyhow::{anyhow, Result};
use sov_modules_api::PublicKey;
use sov_state::WorkingSet;

impl<C: sov_modules_api::Context> Election<C> {
    pub(crate) fn init_module(&self, working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        let admin_pub_key = C::PublicKey::try_from("election_admin")
            .map_err(|_| anyhow!("Admin initialization failed"))?;

        self.admin.set(admin_pub_key.to_address(), working_set);
        self.is_frozen.set(false, working_set);

        Ok(())
    }
}
