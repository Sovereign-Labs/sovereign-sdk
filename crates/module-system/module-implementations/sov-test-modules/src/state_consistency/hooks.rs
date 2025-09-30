use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{BlockHooks, Spec, StateCheckpoint};
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
    }
}
