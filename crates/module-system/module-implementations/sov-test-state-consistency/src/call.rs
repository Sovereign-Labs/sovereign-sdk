use std::fmt::Debug;

use anyhow::Result;
use schemars::JsonSchema;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{Context, EventEmitter, Spec, TxState};
use strum::{EnumDiscriminants, EnumIs, VariantArray};

use super::{Event, StateConsistency};

/// This enumeration represents the available call messages for interacting with the module.
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Eq,
    Clone,
    JsonSchema,
    EnumDiscriminants,
    EnumIs,
    UniversalWallet,
)]
#[serde(rename_all = "snake_case")]
#[strum_discriminants(derive(VariantArray, EnumIs))]
pub enum CallMessage {
    /// Insert a new value into the state, asserting the old one matches the current storage.
    UpdateValue {
        /// The current value in the module. Asserted to ensure state consistency; the transaction
        /// is invalid on mismatch.
        old_check: u64,
        /// The new value to save.
        new: u64,
    },
    /// Updates the value stored in accessory state.
    UpdateAccessoryState(u64),
    /// Assert the state accessor's block and slot properties are as expected.
    AssertBlockState {
        /// The expected visible slot number.
        expected_visible_slot_number: u64,
        /// The expected rollup height.
        expected_rollup_height: u64,
        /// The expected state root.
        expected_state_root: Vec<u8>,
    },
}

impl<S: Spec> StateConsistency<S> {
    pub(crate) fn update_value(
        &mut self,
        old_check: u64,
        new: u64,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        // Get the current value for this sender's address
        let sender_address = context.sender();
        let current_value = self.values.get(sender_address, state)?.unwrap_or(0);

        anyhow::ensure!(
            current_value == old_check,
            "Current value mismatched: stored value {}, transaction expected {}",
            current_value,
            old_check
        );

        // Update the value for this address
        self.values.set(sender_address, &new, state)?;

        // Emit event
        self.emit_event(
            state,
            Event::ValueUpdated {
                address: *sender_address,
                old_value: current_value,
                new_value: new,
            },
        );

        Ok(())
    }

    pub(crate) fn update_accessory_state(
        &mut self,
        new: u64,
        _context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        self.accessory_value.set(&new, state)?;
        // Emit event
        self.emit_event(state, Event::AccessoryValueUpdated { new_value: new });

        Ok(())
    }

    pub(crate) fn assert_block_state(
        &mut self,
        expected_visible_slot_number: u64,
        expected_rollup_height: u64,
        expected_state_root: Vec<u8>,
        _context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let mut mismatches = Vec::<String>::new();

        let rollup_height = state.rollup_height_to_access().get();
        // Sanity check
        let saved_rollup_height = self.latest_rollup_height.get(state)?.unwrap_or(0);
        assert_eq!(rollup_height, saved_rollup_height, "Mismatch between rollup height saved from begin_rollup_block_hook: {saved_rollup_height} and the one read from TxState: {rollup_height}");
        if rollup_height != expected_rollup_height {
            mismatches.push(
                format!(
                    "Rollup height mismatch. Transaction expected {expected_rollup_height}, but actual state is {rollup_height}",
                ));
        };

        let visible_slot_number = state.current_visible_slot_number().get();
        // Sanity check
        let saved_visible_slot_number = self.latest_visible_slot_number.get(state)?.unwrap_or(0);
        assert_eq!(visible_slot_number, saved_visible_slot_number, "Mismatch between visible slot number saved from begin_rollup_block_hook: {saved_visible_slot_number} and the one read from TxState: {visible_slot_number}");
        // Then actual tx assert - transactions should use values from the sequencer; this will
        // expose any mismatch in the kernel state when the node process it afterwards
        if visible_slot_number != expected_visible_slot_number {
            mismatches.push(format!(
            "Visible slot number mismatch. Transaction expected {expected_visible_slot_number}, but actual state is {visible_slot_number}",
        ));
        };

        let max_slot_number = state.max_allowed_slot_number_to_access().get();
        // If we're here then we already checked expected_visible_slot_number matches
        // saved_visible_slot_number and state.visible_slot_number, so no need for sanity check
        if !max_slot_number == expected_visible_slot_number {
            mismatches.push(format!(
            "Max slot number mismatch. Transaction expected {expected_visible_slot_number} (equal to visible_slot_number), but actual state is {max_slot_number}",
        ));
        };

        let state_root = self.latest_state_root.get_or_err(state)??;
        if expected_state_root != state_root.as_ref() {
            mismatches.push(format!(
            "State root mismatch. Transaction expected {}, but actual root (saved in begin_rollup_block_hook, from previous slot) is {}",
            hex::encode(expected_state_root),
            state_root
        ));
        };

        if !mismatches.is_empty() {
            anyhow::bail!(
                "Block state assertion failed at rollup height {}. List of mismatches: {:?}",
                rollup_height,
                mismatches
            )
        }

        // Else, if successful, ncrement successful assertions count
        let current_count = self.successful_assertions_count.get(state)?.unwrap_or(0);
        let new_count = current_count
            .checked_add(1)
            .expect("Overflow when incremeting u64 counter");
        self.successful_assertions_count.set(&new_count, state)?;

        Ok(())
    }
}
