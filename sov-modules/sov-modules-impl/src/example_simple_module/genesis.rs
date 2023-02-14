use super::ValueAdderModule;
use sov_modules_api::Error;

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    pub(crate) fn genesis(&mut self) -> Result<(), Error> {
        let maybe_adimin = C::PublicKey::try_from("admin");

        let admin = match maybe_adimin {
            Ok(admin) => admin,
            Err(_) => return Err("Bad public key for admin")?,
        };

        self.admin.set(admin);

        Ok(())
    }
}
