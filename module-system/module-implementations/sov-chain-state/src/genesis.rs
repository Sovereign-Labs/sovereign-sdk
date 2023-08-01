use anyhow::Result;
use sov_rollup_interface::zk::ValidityCondition;
use sov_state::WorkingSet;

use crate::ChainState;

impl<C: sov_modules_api::Context, Cond: ValidityCondition> ChainState<C, Cond> {
    pub(crate) fn init_module(
        &self,
        _config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.slot_height.set(&0, working_set);
        Ok(())
    }
}
