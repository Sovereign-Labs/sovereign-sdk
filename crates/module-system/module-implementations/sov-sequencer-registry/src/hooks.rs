use std::collections::BTreeSet;

use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{BlockHooks, DaSpec, Spec, StateCheckpoint, VersionReader};

use crate::{CurrentDaAddress, SequencerRegistry};

impl<S: Spec> BlockHooks for SequencerRegistry<S> {
    type Spec = S;

    fn begin_rollup_block_hook(
        &mut self,
        _visible_hash: &<S::Storage as sov_state::Storage>::Root,
        state: &mut StateCheckpoint<S>,
    ) {
        if !Self::rotation_enabled(state) {
            return;
        }
        if self
            .registration_id_counter
            .get(state)
            .unwrap_infallible()
            .is_some()
        {
            return;
        }
        assert_eq!(
            state.rollup_height_to_access().get(),
            config_value!("SEQUENCER_DA_ROTATION_ACTIVATION_HEIGHT"),
            "Sequencer DA rotation tracking was not initialized at activation"
        );

        let mut addresses = BTreeSet::new();
        for text in config_value!("SEQUENCER_DA_ROTATION_UPGRADE_ADDRESSES").split_whitespace() {
            let address = text
                .parse::<<S::Da as DaSpec>::Address>()
                .expect("Invalid sequencer DA upgrade address");
            assert_eq!(
                text,
                address.to_string(),
                "Sequencer DA upgrade addresses must be canonical"
            );
            assert!(
                addresses.insert(text.to_owned()),
                "Duplicate sequencer DA upgrade address"
            );
        }
        let mut owners = BTreeSet::new();
        let mut registrations = Vec::new();
        for text in addresses {
            let da_address = text.parse::<<S::Da as DaSpec>::Address>().unwrap();
            if let Some(known) = self
                .known_sequencers
                .get(&da_address, state)
                .unwrap_infallible()
            {
                assert!(
                    owners.insert(known.address),
                    "Sequencer owners must consolidate before DA rotation activation"
                );
                registrations.push((da_address, known.address));
            }
        }

        let mut registration_id = 0u64;
        for (da_address, owner) in registrations {
            registration_id = registration_id
                .checked_add(1)
                .expect("Sequencer registration IDs exhausted");
            self.current_da_address_by_sequencer
                .set(
                    &owner,
                    &CurrentDaAddress {
                        registration_id,
                        current_da_address: da_address,
                    },
                    state,
                )
                .unwrap_infallible();
        }
        self.registration_id_counter
            .set(&registration_id, state)
            .unwrap_infallible();
    }

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
