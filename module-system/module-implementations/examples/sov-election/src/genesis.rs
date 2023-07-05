use anyhow::Result;
use sov_state::WorkingSet;

use super::Election;

impl<C: sov_modules_api::Context> Election<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.admin.set(&config.admin, working_set);
        self.is_frozen.set(&false, working_set);

        Ok(())
    }
}
