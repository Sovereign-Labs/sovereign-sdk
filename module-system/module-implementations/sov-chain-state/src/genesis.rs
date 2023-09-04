use anyhow::Result;
use sov_state::WorkingSet;

use crate::ChainState;

impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> ChainState<C, Da> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.genesis_height
            .set(&config.initial_slot_height, working_set);

        self.slot_height
            .set(&config.initial_slot_height, working_set);
        Ok(())
    }
}
