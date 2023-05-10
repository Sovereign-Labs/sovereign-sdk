use super::ValueSetter;
use anyhow::Result;
use sov_state::WorkingSet;

impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(
        &self,
        admin_address: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.admin.set(admin_address.clone(), working_set);
        Ok(())
    }
}
