use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::{hooks::SlotHooks, Context, Spec};
use sov_rollup_interface::{da::BlobTransactionTrait, zk::traits::ValidityCondition};

use crate::ChainState;

impl<Ctx: Context, Cond: ValidityCondition + BorshDeserialize + BorshSerialize> SlotHooks
    for ChainState<Ctx, Cond>
{
    type Context = Ctx;

    fn begin_slot_hook(
        &self,
        blob: &mut impl BlobTransactionTrait,
        state_checkpoint: sov_state::StateCheckpoint<
            <Self::Context as sov_modules_api::Spec>::Storage,
        >,
    ) -> anyhow::Result<<Self::Context as Spec>::Storage> {
        let mut working_set = state_checkpoint.to_revertable();
        self.increment_slot_height(&mut working_set);
        // TODO: store state transition
        Ok(working_set.consume_get_storage())
    }
}
