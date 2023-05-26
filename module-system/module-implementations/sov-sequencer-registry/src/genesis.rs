use crate::Sequencer;
use anyhow::Result;
use sov_state::WorkingSet;

impl<C: sov_modules_api::Context> Sequencer<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Genesis>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.seq_rollup_address
            .set(config.seq_rollup_address.clone(), working_set);

        self.seq_da_address
            .set(config.seq_da_address.clone(), working_set);

        self.coins_to_lock
            .set(config.coins_to_lock.clone(), working_set);

        Ok(())
    }
}
