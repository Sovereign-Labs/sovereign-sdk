use anyhow::Result;
use sov_state::WorkingSet;

use crate::SequencerRegistry;

impl<C: sov_modules_api::Context> SequencerRegistry<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.coins_to_lock.set(&config.coins_to_lock, working_set);
        self.register_sequencer(
            config.seq_da_address.clone(),
            &config.seq_rollup_address,
            working_set,
        )?;

        Ok(())
    }
}
