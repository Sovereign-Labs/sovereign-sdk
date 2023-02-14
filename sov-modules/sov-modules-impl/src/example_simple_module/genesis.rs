use super::ValueAdderModule;
use anyhow::{bail, Result};

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(&mut self) -> Result<()> {
        let maybe_admin = C::PublicKey::try_from("admin");

        let admin = match maybe_admin {
            Ok(admin) => admin,
            Err(_) => bail!("Admin initialization failed"),
        };

        self.admin.set(admin);
        Ok(())
    }
}
