use core::fmt;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::hooks::SlotHooks;
use sov_modules_api::{Context, Spec};
use sov_rollup_interface::da::BlobTransactionTrait;
use sov_rollup_interface::zk::ValidityCondition;
use thiserror::Error;

use crate::{ChainState, StateTransitionId};

#[derive(Debug, Clone, Error)]
pub(crate) enum ChainStateError {
    NoTransitionInProgress,
}

impl fmt::Display for ChainStateError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let error_msg = match self {
            Self::NoTransitionInProgress => "No transition in progress",
        };

        write!(f, "{error_msg}")
    }
}

impl<Ctx: Context, Cond: ValidityCondition + BorshDeserialize + BorshSerialize> SlotHooks
    for ChainState<Ctx, Cond>
{
    type Context = Ctx;

    fn begin_slot_hook(
        &self,
        _blob: &mut impl BlobTransactionTrait,
        state_checkpoint: sov_state::StateCheckpoint<
            <Self::Context as sov_modules_api::Spec>::Storage,
        >,
    ) -> anyhow::Result<<Self::Context as Spec>::Storage> {
        let mut working_set = state_checkpoint.to_revertable();

        self.increment_slot_height(&mut working_set);
        Ok(working_set.consume_get_storage())
    }

    fn end_slot_hook(
        &self,
        new_state_root: [u8; 32],
        state_checkpoint: sov_state::StateCheckpoint<
            <Self::Context as sov_modules_api::Spec>::Storage,
        >,
    ) -> anyhow::Result<()> {
        let mut working_set = state_checkpoint.to_revertable();
        let last_transition_in_progress = self
            .in_progress_transition
            .get(&mut working_set)
            .ok_or(ChainStateError::NoTransitionInProgress)?;
        let transition = StateTransitionId {
            da_block_hash: last_transition_in_progress.da_block_hash,
            post_state_root: new_state_root,
            validity_condition: last_transition_in_progress.validity_condition,
        };

        self.store_state_transition(
            self.slot_height(&mut working_set),
            transition,
            &mut working_set,
        );

        Ok(())
    }
}
