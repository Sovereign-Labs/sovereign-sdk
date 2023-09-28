use sov_modules_api::hooks::{FinalizeHook, SlotHooks};
use sov_modules_api::{AccessoryWorkingSet, Context, Spec, WorkingSet};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_state::Storage;

use super::ChainState;
use crate::{StateTransitionId, TransitionInProgress};

impl<C: Context, Da: sov_modules_api::DaSpec> SlotHooks<Da> for ChainState<C, Da> {
    type Context = C;

    fn begin_slot_hook(
        &self,
        slot_header: &Da::BlockHeader,
        validity_condition: &Da::ValidityCondition,
        pre_state_root: &<<Self::Context as Spec>::Storage as Storage>::Root,
        working_set: &mut WorkingSet<C>,
    ) {
        if self.genesis_hash.get(working_set).is_none() {
            // The genesis hash is not set, hence this is the
            // first transition right after the genesis block
            self.genesis_hash.set(pre_state_root, working_set)
        } else {
            let transition: StateTransitionId<
                Da,
                <<Self::Context as Spec>::Storage as Storage>::Root,
            > = {
                let last_transition_in_progress = self
                    .in_progress_transition
                    .get(working_set)
                    .expect("There should always be a transition in progress");

                StateTransitionId {
                    da_block_hash: last_transition_in_progress.da_block_hash,
                    post_state_root: pre_state_root.clone(),
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
        self.time.set(&slot_header.time(), working_set);

        self.in_progress_transition.set(
            &TransitionInProgress {
                da_block_hash: slot_header.hash(),
                validity_condition: *validity_condition,
            },
            working_set,
        );
    }

    fn end_slot_hook(&self, _working_set: &mut WorkingSet<C>) {}
}

impl<C: Context, Da: sov_modules_api::DaSpec> FinalizeHook<Da> for ChainState<C, Da> {
    type Context = C;

    fn finalize_hook(
        &self,
        _root_hash: &<C::Storage as Storage>::Root,
        _accesorry_working_set: &mut AccessoryWorkingSet<C>,
    ) {
    }
}
