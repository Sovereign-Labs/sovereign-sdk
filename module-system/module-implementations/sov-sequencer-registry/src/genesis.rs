use anyhow::Result;
use sov_modules_api::{BlobReaderTrait, Context, DaSpec};
use sov_state::WorkingSet;

use crate::SequencerRegistry;

impl<C: Context, Da: DaSpec> SequencerRegistry<C, Da>
where
    <<Da as DaSpec>::BlobTransaction as BlobReaderTrait>::Address:
        borsh::BorshSerialize + borsh::BorshDeserialize,
{
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
        if config.is_preferred_sequencer {
            self.preferred_sequencer
                .set(&config.seq_da_address, working_set);
        }

        Ok(())
    }
}
