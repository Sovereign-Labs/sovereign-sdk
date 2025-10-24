use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{BlockHooks, Spec, StateCheckpoint, VersionReader};
use sov_state::Storage;

use super::StateConsistency;

impl<S: Spec> BlockHooks for StateConsistency<S> {
    type Spec = S;
    fn begin_rollup_block_hook(
        &mut self,
        visible_hash: &<<Self::Spec as Spec>::Storage as Storage>::Root,
        state: &mut StateCheckpoint<Self::Spec>,
    ) {
        self.latest_state_root
            .set(visible_hash, state)
            .unwrap_infallible();

        // Store the current visible slot number
        let visible_slot_number = state.current_visible_slot_number();
        self.latest_visible_slot_number
            .set(&visible_slot_number.get(), state)
            .unwrap_infallible();

        // Store the current rollup height
        let rollup_height = state.rollup_height_to_access();
        self.latest_rollup_height
            .set(&rollup_height.get(), state)
            .unwrap_infallible();
    }
}
