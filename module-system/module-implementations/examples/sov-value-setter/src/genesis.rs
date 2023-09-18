use anyhow::Result;
use sov_modules_api::WorkingSet;

use super::ValueSetter;

impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(
        &self,
        admin_config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        self.admin.set(&admin_config.admin, working_set);
        Ok(())
    }
}
