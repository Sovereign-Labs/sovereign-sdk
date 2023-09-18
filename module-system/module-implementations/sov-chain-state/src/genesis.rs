use anyhow::Result;
use sov_modules_api::WorkingSet;

use crate::ChainState;

impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> ChainState<C, Da> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        self.genesis_height
            .set(&config.initial_slot_height, working_set);

        self.slot_height
            .set(&config.initial_slot_height, working_set);

        self.time.set(&config.current_time, working_set);
        Ok(())
    }
}
