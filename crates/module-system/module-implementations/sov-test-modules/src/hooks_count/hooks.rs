//! Implements the `BlockHooks` trait for the `HooksCount` module.
//!
//! These hook simply count the number of times they are called. This is useful for testing and debugging.

use sov_modules_api::prelude::UnwrapInfallible;
#[cfg(feature = "native")]
use sov_modules_api::{AccessoryStateReaderAndWriter, FinalizeHook};
use sov_modules_api::{BlockHooks, Spec, StateCheckpoint};
use sov_state::Storage;

use super::HooksCount;

impl<S: Spec> BlockHooks for HooksCount<S> {
    type Spec = S;
    /// Hook that runs at the beginning of the `apply_slot` function inside the `StateTransitionFunction`.
    fn begin_rollup_block_hook(
        &mut self,
        visible_hash: &<<Self::Spec as Spec>::Storage as Storage>::Root,
        state: &mut StateCheckpoint<Self::Spec>,
    ) {
        let next_value = self
            .begin_rollup_block_hook_count
            .get(state)
            .unwrap_infallible()
            .unwrap_or(0)
            + 1;
        self.begin_rollup_block_hook_count
            .set(&next_value, state)
            .unwrap_infallible();
        self.latest_state_root
            .set(visible_hash, state)
            .unwrap_infallible();
    }

    /// Hook that runs at the end of the `apply_slot` function inside the `StateTransitionFunction`.
    fn end_rollup_block_hook(&mut self, state: &mut StateCheckpoint<Self::Spec>) {
        let next_value = self
            .end_rollup_block_hook_count
            .get(state)
            .unwrap_infallible()
            .unwrap_or(0)
            + 1;
        self.end_rollup_block_hook_count
            .set(&next_value, state)
            .unwrap_infallible();
    }
}

#[cfg(feature = "native")]
impl<S: Spec> FinalizeHook for HooksCount<S> {
    type Spec = S;
    fn finalize_hook(
        &mut self,
        _root_hash: &<<Self::Spec as Spec>::Storage as Storage>::Root,
        state: &mut impl AccessoryStateReaderAndWriter,
    ) {
        let next_value = self
            .finalize_hook_count
            .get(state)
            .unwrap_infallible()
            .unwrap_or(0)
            + 1;
        self.finalize_hook_count
            .set(&next_value, state)
            .unwrap_infallible();
    }
}
