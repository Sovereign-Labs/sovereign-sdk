use std::fmt::Debug;

use anyhow::Result;
use schemars::JsonSchema;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{Context, Spec, TxState, EventEmitter};
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
        new: u64
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
        _context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let old_value = self.value.get(state)?.ok_or(anyhow::anyhow!(
            "Empty state `value`, should not be possible"
        ))?;
        anyhow::ensure!(
            old_value == old_check,
            "Current value mismatched: stored value {old_value}, transaction expected {old_check}"
        );

        self.value.set(&new, state)?;

        self.emit_event(
            state,Event::ValueUpdated {
            old_value,
            new_value: new,
            });

        Ok(())
    }

    pub(crate) fn update_accessory_state(
        &mut self,
        new: u64,
        _context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        Ok(self.accessory_value.set(&new, state)?)
    }

    pub(crate) fn assert_block_state(
        &self,
        expected_visible_slot_number: u64,
        expected_rollup_height: u64,
        expected_state_root: Vec<u8>,
        _context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let visible_slot_number = state.current_visible_slot_number();
        anyhow::ensure!(
            visible_slot_number.get() == expected_visible_slot_number,
            "Visible slot number is not as expected. Expected {}, but got {}",
            expected_visible_slot_number,
            visible_slot_number.get()
        );

        let rollup_height = state.rollup_height_to_access();
        anyhow::ensure!(
            rollup_height.get() == expected_rollup_height,
            "Rollup height is not as expected. Expected {}, but got {}",
            expected_rollup_height,
            rollup_height.get()
        );

        let max_slot_number = state.max_allowed_slot_number_to_access();
        anyhow::ensure!(
            max_slot_number.get() == expected_visible_slot_number,
            "Max slot number is not as expected. Expected {} (equal to visible_slot_number), but got {}",
            expected_visible_slot_number,
            max_slot_number.get()
        );

        let state_root = self.latest_state_root.get_or_err(state)??;
        anyhow::ensure!(
            expected_state_root == state_root.as_ref(),
            "State root is not as expected for height {}. Expected {}, but got {}",
            state.rollup_height_to_access(),
            hex::encode(expected_state_root),
            state_root
        );
        Ok(())
    }
}
