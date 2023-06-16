use crate::Sequencer;
use sov_modules_api::{hooks::ApplyBlobHooks, Context};
use sov_rollup_interface::da::BlobTransactionTrait;
use sov_state::WorkingSet;

impl<C: Context> ApplyBlobHooks for Sequencer<C> {
    type Context = C;
    type BlobResult = u64;

    fn begin_blob_hook(
        &self,
        blob: &mut impl BlobTransactionTrait,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        let next_sequencer_da = self.seq_da_address.get_or_err(working_set);

        match next_sequencer_da {
            Ok(next_sequencer_da) => {
                if next_sequencer_da != blob.sender().as_ref() {
                    anyhow::bail!("Invalid next sequencer.")
                }
            }
            Err(_) => anyhow::bail!("Sequencer {:?} not registered. ", blob.sender()),
        }

        let sequencer = &self.seq_rollup_address.get_or_err(working_set)?;
        let locker = &self.address;
        let coins = self.coins_to_lock.get_or_err(working_set)?;

        self.bank
            .transfer_from(sequencer, locker, coins, working_set)?;

        Ok(())
    }

    fn end_blob_hook(
        &self,
        _result: Self::BlobResult,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        let sequencer = &self.seq_rollup_address.get_or_err(working_set)?;
        let locker = &self.address;
        let coins = self.coins_to_lock.get_or_err(working_set)?;

        self.bank
            .transfer_from(locker, sequencer, coins, working_set)?;

        Ok(())
    }
}
