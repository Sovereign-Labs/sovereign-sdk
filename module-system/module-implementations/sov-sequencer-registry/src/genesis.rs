use crate::SequencerRegistry;
use anyhow::Result;
use sov_state::WorkingSet;

impl<C: sov_modules_api::Context> SequencerRegistry<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.seq_rollup_address
            .set(&config.seq_rollup_address, working_set);

        self.seq_da_address.set(&config.seq_da_address, working_set);

        self.coins_to_lock.set(&config.coins_to_lock, working_set);

        self.allowed_sequencers.set(
            &config.seq_da_address,
            &config.seq_rollup_address,
            working_set,
        );

        Ok(())
    }
}
