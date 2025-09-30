//! Implements call message generation for the [`sov_test_modules::state_consistency::StateConsistency`] module.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sov_modules_api::prelude::arbitrary;
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::{CryptoSpec, PrivateKey, PublicKey, Spec};
use strum::EnumDiscriminants;

use crate::generators::basic::BasicClientConfig;
use crate::interface::{
    CallMessageGenerator, GeneratedMessage, GeneratorState, MessageOutcome, MessageValidity,
    TagAction, Taggable,
};
use crate::state::{AccountState, ApplyToState};
use crate::ChangelogEntry;

mod harness_interface;
pub use harness_interface::*;

/// The state of a state consistency test account
#[derive(Debug, Clone)]
pub struct StateConsistencyAccount {
    pub(crate) current_value: u64,
}

impl<S: Spec, T> From<&AccountState<S, T>> for StateConsistencyAccount {
    fn from(value: &AccountState<S, T>) -> StateConsistencyAccount {
        StateConsistencyAccount {
            current_value: value.consistency_value,
        }
    }
}

impl<S: Spec, T> ApplyToState<S, T> for StateConsistencyAccount {
    fn apply_to(self, account: &mut AccountState<S, T>) {
        account.consistency_value = self.current_value;
    }
}

impl Taggable for StateConsistencyAccount {
    type Tag = ();

    fn take_tags(&mut self) -> impl IntoIterator<Item = TagAction<Self::Tag>> {
        vec![].into_iter()
    }

    fn add_tag(&mut self, _tag: Self::Tag) {}

    fn remove_tag(&mut self, _tag: Self::Tag) {}
}

/// A message generator for the `StateConsistency` module.
#[derive(Debug, Clone)]
pub struct StateConsistencyMessageGenerator<S: Spec> {
    /// The private key to use for sending transactions (also serves as singleton account key)
    test_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
}

impl<S: Spec> StateConsistencyMessageGenerator<S> {
    /// Creates a new [`StateConsistencyMessageGenerator`]
    pub fn new(test_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey) -> Self {
        Self { test_key }
    }

    /// Get the test account address (derived from test key)
    fn test_address(&self) -> S::Address {
        self.test_key.pub_key().credential_id().into()
    }

    /// Gets or creates the test account for this generator
    fn get_or_create_test_account(
        &self,
        generator_state: &mut impl GeneratorState<S, AccountView = StateConsistencyAccount>,
    ) -> (S::Address, StateConsistencyAccount) {
        let address = self.test_address();

        if let Some(account) = generator_state.get_account(&address) {
            (address, account)
        } else {
            // Create new test account
            let account = StateConsistencyAccount {
                current_value: 0, // TODO: query from state on init?
            };

            // Save it immediately so it exists in the generator state
            generator_state.update_account(&address, account.clone());
            (address, account)
        }
    }
}

/// A complete description of any possible state change created by the [`StateConsistencyMessageGenerator`].
#[derive(Debug, Clone, Deserialize, Serialize, EnumDiscriminants, PartialEq)]
#[serde(rename_all = "snake_case")]
#[strum_discriminants(name(StateConsistencyChangeLogDiscriminant), derive(Hash))]
pub enum StateConsistencyChangeLogEntry {
    /// The value was updated
    ValueUpdated {
        /// The new value stored in state
        new_value: u64,
    },
}

/// Client configuration for state consistency checks
pub struct StateConsistencyClientConfig {}

impl From<BasicClientConfig> for StateConsistencyClientConfig {
    fn from(_config: BasicClientConfig) -> Self {
        Self {}
    }
}

#[async_trait]
impl ChangelogEntry for StateConsistencyChangeLogEntry {
    type ClientConfig = StateConsistencyClientConfig;
    type Discriminant = StateConsistencyChangeLogDiscriminant;

    async fn assert_state(
        &self,
        _rollup_state_accessor: Arc<Self::ClientConfig>,
    ) -> Result<(), anyhow::Error> {
        // Module asserts state internally and rejects transactions that aren't valid updates
        Ok(())
    }

    fn as_discriminant(&self) -> Self::Discriminant {
        match self {
            StateConsistencyChangeLogEntry::ValueUpdated { .. } => {
                StateConsistencyChangeLogDiscriminant::ValueUpdated
            }
        }
    }
}

#[async_trait]
impl<S: Spec> CallMessageGenerator<S> for StateConsistencyMessageGenerator<S> {
    type Module = sov_test_modules::state_consistency::StateConsistency<S>;
    type AccountView = StateConsistencyAccount;
    type ChangelogEntry = StateConsistencyChangeLogEntry;
    type Tag = ();

    fn generate_setup_messages(
        &self,
        _u: &mut arbitrary::Unstructured<'_>,
        _generator_state: &mut impl GeneratorState<S, AccountView = Self::AccountView>,
    ) -> arbitrary::Result<
        Vec<
            GeneratedMessage<
                S,
                sov_test_modules::state_consistency::CallMessage,
                Self::ChangelogEntry,
            >,
        >,
    > {
        // No setup
        Ok(vec![])
    }

    fn generate_call_message(
        &self, // Note: &self, not &mut self!
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = Self::AccountView>,
        validity: MessageValidity,
    ) -> arbitrary::Result<
        GeneratedMessage<S, sov_test_modules::state_consistency::CallMessage, Self::ChangelogEntry>,
    > {
        use sov_test_modules::state_consistency::CallMessage;

        // Get the test account for this generator
        let (test_addr, mut test_account) = self.get_or_create_test_account(generator_state);

        match validity {
            MessageValidity::Valid => {
                // Generate UpdateValue with correct old_check for consistency
                let old_value = test_account.current_value;
                let new_value = u.int_in_range(0..=u64::MAX)?;

                // Update the account's tracked value
                test_account.current_value = new_value;

                // Save the updated account back to generator state
                generator_state.update_account(&test_addr, test_account);

                Ok(GeneratedMessage::new(
                    CallMessage::UpdateValue {
                        old_check: old_value,
                        new: new_value,
                    },
                    self.test_key.clone(),
                    MessageOutcome::Successful {
                        changes: vec![StateConsistencyChangeLogEntry::ValueUpdated { new_value }],
                    },
                ))
            }
            MessageValidity::Invalid => {
                // Generate with intentionally wrong old_check
                let current_value = test_account.current_value;
                let wrong_old = current_value.wrapping_add(12345); // Guaranteed wrong
                let new_value = u.int_in_range(0..=u64::MAX)?;

                // Don't update the account since this will revert
                Ok(GeneratedMessage::new(
                    CallMessage::UpdateValue {
                        old_check: wrong_old,
                        new: new_value,
                    },
                    self.test_key.clone(),
                    MessageOutcome::Reverted,
                ))
            }
        }
    }
}
