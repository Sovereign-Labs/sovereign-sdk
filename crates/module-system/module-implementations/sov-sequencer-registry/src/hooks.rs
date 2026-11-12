use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{BlockHooks, Spec, StateCheckpoint};

use crate::SequencerRegistry;

impl<S: Spec> BlockHooks for SequencerRegistry<S> {
    type Spec = S;

    fn end_rollup_block_hook(&mut self, state: &mut StateCheckpoint<Self::Spec>) {
        if !Self::rotation_enabled(state) {
            return;
        }
        let Some(pending) = self
            .pending_da_address_update
            .remove(state)
            .unwrap_infallible()
        else {
            return;
        };

        // `update_da_address` validated both addresses earlier in this block. The entry at the
        // old address can only disappear through `withdraw` (refused while this update is
        // pending), `slash_sequencer` (which drops the pending update) or another rotation
        // (refused while one is pending), and the new address is reserved against
        // registration. Anything else is a bug, and applying the move anyway would lose or
        // misattribute the sequencer's stake.
        let existing_sequencer = self
            .known_sequencers
            .get(&pending.old_da_address, state)
            .unwrap_infallible()
            .expect("pending DA rotation invariant violated: old address disappeared before end-of-block");

        assert_eq!(
            existing_sequencer.address, pending.sequencer,
            "pending DA rotation invariant violated: old address changed owner before end-of-block"
        );
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
