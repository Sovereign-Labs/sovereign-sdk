use anyhow::Result;
use sov_state::WorkingSet;

use crate::IbcRouterModule;

impl<C: sov_modules_api::Context> IbcRouterModule<C> {
    pub(crate) fn init_module(
        &self,
        _config: &<Self as sov_modules_api::Module>::Config,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        Ok(())
    }
}
