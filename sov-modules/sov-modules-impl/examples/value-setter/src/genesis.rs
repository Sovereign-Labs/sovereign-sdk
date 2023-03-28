use super::ValueSetter;
use anyhow::{anyhow, Result};
use sov_modules_api::PublicKey;
use sov_state::WorkingSet;

impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(&self, working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        let admin_pub_key = C::PublicKey::try_from("value_setter_admin")
            .map_err(|_| anyhow!("Admin initialization failed"))?;

        self.admin
            .set(admin_pub_key.to_address::<C::Address>(), working_set);
        Ok(())
    }
}
