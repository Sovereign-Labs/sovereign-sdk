use super::ValueSetter;
use anyhow::{bail, Result};
use sov_state::WorkingSet;

impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(&mut self, working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        let maybe_admin = C::PublicKey::try_from("admin");

        let admin = match maybe_admin {
            Ok(admin) => admin,
            Err(_) => bail!("Admin initialization failed"),
        };

        self.admin.set(admin, working_set);
        Ok(())
    }
}
