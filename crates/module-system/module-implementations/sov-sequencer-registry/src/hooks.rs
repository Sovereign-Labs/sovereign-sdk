use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{BlockHooks, Spec, StateCheckpoint};

use crate::SequencerRegistry;

impl<S: Spec> BlockHooks for SequencerRegistry<S> {
    type Spec = S;

    fn end_rollup_block_hook(&mut self, state: &mut StateCheckpoint<Self::Spec>) {
        let Some(pending) = self
            .pending_da_address_update
            .remove(state)
            .unwrap_infallible()
        else {
            return;
        };

        // Scheduling validated both addresses; if these fail, state changed in an
        // unexpected way between transaction execution and the end-block hook.
        let existing_sequencer = self
            .known_sequencers
            .get(&pending.old_da_address, state)
            .unwrap_infallible()
            .expect("pending DA rotation invariant violated: old address disappeared before end-of-block");

        assert!(
            self
                .known_sequencers
                .get(&pending.new_da_address, state)
                .unwrap_infallible()
                .is_none(),
            "pending DA rotation invariant violated: new address became registered before end-of-block",
        );

        self.apply_da_rotation(
            &pending.old_da_address,
            &pending.new_da_address,
            &existing_sequencer,
            state,
        )
        .unwrap_infallible();
    }
}
