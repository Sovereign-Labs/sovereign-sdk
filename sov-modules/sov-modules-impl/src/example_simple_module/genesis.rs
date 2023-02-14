use super::ValueAdderModule;
use sov_modules_api::ModuleError;

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    pub(crate) fn init_module(&mut self) -> Result<(), ModuleError> {
        let maybe_admin = C::PublicKey::try_from("admin");

        let admin = match maybe_admin {
            Ok(admin) => admin,
            Err(_) => return Err("Admin initialization failed")?,
        };

        self.admin.set(admin);
        Ok(())
    }
}
