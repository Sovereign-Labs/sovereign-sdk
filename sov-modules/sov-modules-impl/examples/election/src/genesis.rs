use super::Election;
use anyhow::Result;
use sov_state::WorkingSet;

impl<C: sov_modules_api::Context> Election<C> {
    pub(crate) fn init_module(
        &self,
        admin: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.admin.set(admin.clone(), working_set);
        self.is_frozen.set(false, working_set);

        Ok(())
    }
}
