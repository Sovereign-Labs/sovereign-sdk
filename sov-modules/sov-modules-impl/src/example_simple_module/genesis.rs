use super::ValueAdderModule;
use sov_modules_api::ModuleError;

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    pub(crate) fn genesis(&mut self) -> Result<(), ModuleError> {
        let maybe_admin = C::PublicKey::try_from("admin");

        let admin = match maybe_admin {
            Ok(admin) => admin,
            Err(_) => return Err("Bad public key for admin")?,
        };

        self.admin.set(admin);
        Ok(())
    }
}
