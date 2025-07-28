use std::fmt::Debug;

use anyhow::Result;
use schemars::JsonSchema;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{Context, Spec, TxState};
use strum::{EnumDiscriminants, EnumIs, VariantArray};

use super::HooksCount;

/// This enumeration represents the available call messages for interacting with the `sov-test-modules` module.
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
    /// Assert the visible slot number is as expected.
    AssertVisibleSlotNumber {
        /// The expected visible slot number.
        expected_visible_slot_number: u64,
    },
    /// Assert the state root matches the expected value.
    AssertStateRoot {
        /// The expected state root.
        expected_state_root: Vec<u8>,
    },
    /// A call message that will be delayed.
    DelayedCallMsg,
}

impl<S: Spec> HooksCount<S> {
    pub(crate) fn assert_visible_slot_number(
        &self,
        expected_visible_slot_number: u64,
        _context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let visible_height = state.current_visible_slot_number();
        anyhow::ensure!(
            visible_height.get() == expected_visible_slot_number,
            "Visible height is not as expected. Expected {}, but got {}",
            expected_visible_slot_number,
            visible_height.get()
        );
        Ok(())
    }

    /// Assert the state root is as expected.
    pub(crate) fn assert_state_root(
        &self,
        expected_state_root: Vec<u8>,
        _context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let state_root = self.latest_state_root.get_or_err(state)??;
        anyhow::ensure!(
            expected_state_root == state_root.as_ref(),
            "State root is not as expected. Expected {}, but got {}",
            hex::encode(expected_state_root),
            state_root
        );
        Ok(())
    }
}
