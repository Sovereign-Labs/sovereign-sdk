//! Implements call message generation for the [`sov_test_modules::state_consistency::StateConsistency`] module.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sov_modules_api::prelude::arbitrary;
use sov_modules_api::prelude::arbitrary::Arbitrary;
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::{CryptoSpec, PrivateKey, Spec};
use strum::EnumDiscriminants;

use crate::generators::basic::BasicClientConfig;
use crate::interface::{
    CallMessageGenerator, Distribution, GeneratedMessage, GeneratorState, MessageValidity,
    MessageOutcome, Percent, TagAction, Taggable,
};
use crate::state::{AccountState, ApplyToState};
use crate::ChangelogEntry;

mod harness_interface;
pub use harness_interface::*;

/// Tags used for state consistency accounts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateConsistencyTag {
    /// The singleton account that tracks the global state value
    GlobalStateSingleton,
}

/// Additional data stored in the singleton account to track global state
#[derive(Debug, Clone, Default)]
pub struct StateConsistencyData {
    /// The current global value being tracked
    pub current_value: u64,
}

/// The state of a state consistency test account
#[derive(Debug, Clone)]
pub struct StateConsistencyAccount<S: Spec> {
    pub(crate) private_key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
    pub(crate) current_value: u64,
    pub(crate) tag_changes: Vec<TagAction<StateConsistencyTag>>,
}

impl<S: Spec> From<&AccountState<S, StateConsistencyData>> for StateConsistencyAccount<S> {
    fn from(value: &AccountState<S, StateConsistencyData>) -> StateConsistencyAccount<S> {
        StateConsistencyAccount {
            private_key: value.private_key.clone(),
            current_value: value.additional_info.current_value,
            tag_changes: Vec::new(),
        }
    }
}

impl<S: Spec> ApplyToState<S, StateConsistencyData> for StateConsistencyAccount<S> {
    fn apply_to(self, account: &mut AccountState<S, StateConsistencyData>) {
        account.additional_info.current_value = self.current_value;
    }
}

impl<S: Spec> Taggable for StateConsistencyAccount<S> {
    type Tag = StateConsistencyTag;

    fn add_tag(&mut self, tag: Self::Tag) {
        self.tag_changes.push(TagAction::Add(tag));
    }

    fn remove_tag(&mut self, tag: Self::Tag) {
        self.tag_changes.push(TagAction::Remove(tag));
    }

    fn take_tags(&mut self) -> impl IntoIterator<Item = TagAction<Self::Tag>> {
        std::mem::take(&mut self.tag_changes)
    }
}

/// A message generator for the `StateConsistency` module.
#[derive(Debug, Clone)]
pub struct StateConsistencyMessageGenerator<S: Spec> {
    /// The private key to use for sending transactions (also serves as singleton account key)
    test_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
}

impl<S: Spec> StateConsistencyMessageGenerator<S> {
    /// Creates a new [`StateConsistencyMessageGenerator`]
    pub fn new(
        test_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    ) -> Self {
        Self { test_key }
    }

    /// Get the singleton account address (derived from test key)
    fn singleton_address(&self) -> S::Address {
        self.test_key.pub_key().credential_id().into()
    }

    /// Gets or creates the singleton account that tracks global state
    fn get_or_create_singleton(
        &self,
        generator_state: &mut impl GeneratorState<
            S,
            AccountView = StateConsistencyAccount<S>,
            Tag: From<StateConsistencyTag>,
        >,
    ) -> (S::Address, StateConsistencyAccount<S>) {
        let address = self.singleton_address();

        if let Some(account) = generator_state.get_account(&address) {
            (address, account)
        } else {
            // Create new singleton account
            let mut account = StateConsistencyAccount {
                private_key: self.test_key.clone(),
                current_value: 0, // Start from 0
                tag_changes: Vec::new(),
            };
            account.add_tag(StateConsistencyTag::GlobalStateSingleton);

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
    type AccountView = StateConsistencyAccount<S>;
    type ChangelogEntry = StateConsistencyChangeLogEntry;
    type Tag = ();

    fn generate_setup_messages(
        &self,
        _u: &mut arbitrary::Unstructured<'_>,
        _generator_state: &mut impl GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
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
        generator_state: &mut impl GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
        validity: MessageValidity,
    ) -> arbitrary::Result<
        GeneratedMessage<
            S,
            sov_test_modules::state_consistency::CallMessage,
            Self::ChangelogEntry,
        >,
    > {
        use sov_test_modules::state_consistency::CallMessage;

        // Get the singleton account that tracks our global state
        let (singleton_addr, mut singleton_account) = self.get_or_create_singleton(generator_state);

        match validity {
            MessageValidity::Valid => {
                // Generate UpdateValue with correct old_check for consistency
                let old_value = singleton_account.current_value;
                let new_value = u.int_in_range(0..=u64::MAX)?;

                // Update the singleton's tracked value
                singleton_account.current_value = new_value;

                // Save the updated singleton back to generator state
                generator_state.update_account(&singleton_addr, singleton_account);

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
                let current_value = singleton_account.current_value;
                let wrong_old = current_value.wrapping_add(12345); // Guaranteed wrong
                let new_value = u.int_in_range(0..=u64::MAX)?;

                // Don't update the singleton since this will revert
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
