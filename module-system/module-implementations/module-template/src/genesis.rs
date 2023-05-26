use anyhow::Result;
use sov_state::WorkingSet;

use crate::ExampleModule;

impl<C: sov_modules_api::Context> ExampleModule<C> {
    pub(crate) fn init_module(
        &self,
        _config: &<Self as sov_modules_api::Genesis>::Config,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        Ok(())
    }
}
