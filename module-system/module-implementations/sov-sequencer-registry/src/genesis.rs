use anyhow::Result;
use sov_rollup_interface::da::BlobReaderTrait;
use sov_state::WorkingSet;

use crate::SequencerRegistry;

impl<C: sov_modules_api::Context, B: BlobReaderTrait> SequencerRegistry<C, B>
where
    B::Address: borsh::BorshSerialize + borsh::BorshDeserialize,
{
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.coins_to_lock.set(&config.coins_to_lock, working_set);
        // TODO: Remove on next iteration
        let d = B::Address::try_from(&config.seq_da_address[..])?;
        // self.register_sequencer(&config.seq_rollup_address, working_set)?;
        let address = config.seq_da_address.clone();
        self.register_sequencer(address, &config.seq_rollup_address, working_set)?;
        if config.is_preferred_sequencer {
            self.preferred_sequencer.set(&d, working_set);
        }

        Ok(())
    }
}
