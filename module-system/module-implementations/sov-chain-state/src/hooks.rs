use core::fmt;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::hooks::SlotHooks;
use sov_modules_api::{Context, Spec};
use sov_rollup_interface::services::da::SlotData;
use sov_rollup_interface::zk::ValidityCondition;
use sov_state::{Storage, WorkingSet};
use thiserror::Error;

use super::ChainState;
use crate::{StateTransitionId, TransitionInProgress};

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

impl<Ctx: Context, Cond: ValidityCondition> SlotHooks<Cond> for ChainState<Ctx, Cond> {
    type Context = Ctx;

    fn begin_slot_hook(
        &self,
        slot: &impl SlotData<Condition = Cond>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        let curr_height = self.slot_height.get_or_err(working_set)?;

        if curr_height == 0 {
            // First transition right after the genesis block
            self.genesis_hash.set(
                &working_set.backing().get_state_root(&Default::default())?,
                working_set,
            )
        } else {
            let transition: StateTransitionId<Cond> = {
                let last_transition_in_progress = self
                    .in_progress_transition
                    .get(working_set)
                    .ok_or(ChainStateError::NoTransitionInProgress)?;

                StateTransitionId {
                    da_block_hash: last_transition_in_progress.da_block_hash,
                    post_state_root: working_set.backing().get_state_root(&Default::default())?,
                    validity_condition: last_transition_in_progress.validity_condition,
                }
            };

            self.store_state_transition(
                self.slot_height
                    .get(working_set)
                    .expect("Block height must be set"),
                transition,
                working_set,
            );
        }

        self.increment_slot_height(working_set);
        let validity_condition = slot.validity_condition();

        self.in_progress_transition.set(
            &TransitionInProgress {
                da_block_hash: slot.hash(),
                validity_condition,
            },
            working_set,
        );

        Ok(())
    }
}
