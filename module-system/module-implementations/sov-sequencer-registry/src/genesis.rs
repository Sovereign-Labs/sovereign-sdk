use anyhow::Result;
use sov_modules_api::WorkingSet;

use crate::SequencerRegistry;

impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> SequencerRegistry<C, Da> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        self.coins_to_lock.set(&config.coins_to_lock, working_set);
        self.register_sequencer(
            &config.seq_da_address,
            &config.seq_rollup_address,
            working_set,
        )?;
        if config.is_preferred_sequencer {
            self.preferred_sequencer
                .set(&config.seq_da_address, working_set);
        }

        Ok(())
    }
}
