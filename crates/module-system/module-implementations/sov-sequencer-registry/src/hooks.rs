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

        // The two failure paths below should be unreachable. `update_da_address`
        // verified `old` was registered; nothing between scheduling and end-of-block
        // removes `known_sequencers` entries (slashing happens during batch admission,
        // before tx execution). `ensure_not_pending_new_da_address` + the retired
        // check block any path that could occupy `new` during the same block.
        //
        // `DaAddressUpdated` was already emitted at scheduling time (see
        // `update_da_address`), so a silent failure here would leave the event log
        // out of sync with state. `debug_assert!(false)` makes tests fail loudly if
        // a future change makes either branch reachable; release builds degrade
        // gracefully via `tracing::error!`.
        let Some(existing_sequencer) = self
            .known_sequencers
            .get(&pending.old_da_address, state)
            .unwrap_infallible()
        else {
            debug_assert!(
                false,
                "pending DA rotation invariant violated: old address {} disappeared before end-of-block",
                pending.old_da_address,
            );
            tracing::error!(
                old_da_address = %pending.old_da_address,
                new_da_address = %pending.new_da_address,
                "Pending DA address update could not be applied because the old address is no longer registered"
            );
            return;
        };

        if self
            .known_sequencers
            .get(&pending.new_da_address, state)
            .unwrap_infallible()
            .is_some()
        {
            debug_assert!(
                false,
                "pending DA rotation invariant violated: new address {} became occupied before end-of-block",
                pending.new_da_address,
            );
            tracing::error!(
                old_da_address = %pending.old_da_address,
                new_da_address = %pending.new_da_address,
                "Pending DA address update could not be applied because the new address became registered"
            );
            return;
        }

        self.apply_da_rotation(
            &pending.old_da_address,
            &pending.new_da_address,
            &existing_sequencer,
            state,
        )
        .unwrap_infallible();
    }
}
