//! Implements call message generation for the [`sov_test_state_consistency::StateConsistency`] module.

use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sov_modules_api::prelude::arbitrary;
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
pub struct StateConsistencyAccount<S: Spec> {
    private_key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
    current_value: u64,
    already_tagged: bool,
}

impl<S: Spec, T> From<&AccountState<S, T>> for StateConsistencyAccount<S> {
    fn from(value: &AccountState<S, T>) -> StateConsistencyAccount<S> {
        StateConsistencyAccount {
            private_key: value.private_key.clone(),
            current_value: value.consistency_value,
            // The generator normally uses only one singleton account. The tag is applied once, the
            // first time the account is generated. To avoid a bit of extra overhead in the
            // generator, we can skip the TagAction::Add except the first time we generate the
            // account.
            // So by default, on every normal account load, this will be true. When we do not find
            // an account with the tag and generated a new one for the first time, then we
            // explicitly set it to false.
            already_tagged: true,
        }
    }
}

impl<S: Spec, T> ApplyToState<S, T> for StateConsistencyAccount<S> {
    fn apply_to(self, account: &mut AccountState<S, T>) {
        account.consistency_value = self.current_value;
    }
}

/// Used purely to mark a single account as being the account under test.
#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct StateConsistencyTag;

impl<S: Spec> Taggable for StateConsistencyAccount<S> {
    type Tag = StateConsistencyTag;

    fn take_tags(&mut self) -> impl IntoIterator<Item = TagAction<Self::Tag>> {
        if !self.already_tagged {
            vec![TagAction::Add(StateConsistencyTag)]
        } else {
            vec![]
        }
    }

    fn add_tag(&mut self, _tag: Self::Tag) {}

    fn remove_tag(&mut self, _tag: Self::Tag) {}
}

/// A message generator for the `StateConsistency` module.
#[derive(Debug, Clone, Default)]
pub struct StateConsistencyMessageGenerator<S> {
    _phantom: PhantomData<S>,
}

impl<S: Spec> StateConsistencyMessageGenerator<S> {
    /// Gets or creates the test account for this generator
    fn get_or_create_test_account(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<
            S,
            AccountView = StateConsistencyAccount<S>,
            Tag: From<StateConsistencyTag>,
        >,
    ) -> arbitrary::Result<(S::Address, StateConsistencyAccount<S>)> {
        if let Some(account) = generator_state.get_account_with_tag(StateConsistencyTag.into()) {
            let address = account.private_key.pub_key().credential_id().into();
            Ok((address, account))
        } else {
            let (address, mut account) = generator_state.generate_account(u)?;
            // Force the tag to be added when saving, so it can be retrieved next time
            account.already_tagged = false;
            Ok((address, account))
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
    type Module = sov_test_state_consistency::StateConsistency<S>;
    type AccountView = StateConsistencyAccount<S>;
    type ChangelogEntry = StateConsistencyChangeLogEntry;
    type Tag = StateConsistencyTag;

    fn generate_setup_messages(
        &self,
        _u: &mut arbitrary::Unstructured<'_>,
        _generator_state: &mut impl GeneratorState<S>,
    ) -> arbitrary::Result<
        Vec<GeneratedMessage<S, sov_test_state_consistency::CallMessage, Self::ChangelogEntry>>,
    > {
        // No setup
        Ok(vec![])
    }

    fn generate_call_message(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
        validity: MessageValidity,
    ) -> arbitrary::Result<
        GeneratedMessage<S, sov_test_state_consistency::CallMessage, Self::ChangelogEntry>,
    > {
        use sov_test_state_consistency::CallMessage;

        // Get the test account for this generator
        let (test_addr, mut test_account) = self.get_or_create_test_account(u, generator_state)?;
        let test_key = test_account.private_key.clone();

        match validity {
            MessageValidity::Valid => {
                let old_value = test_account.current_value;
                let new_value = u.int_in_range(0..=u64::MAX)?;

                test_account.current_value = new_value;
                generator_state.update_account(&test_addr, test_account);

                Ok(GeneratedMessage::new(
                    CallMessage::UpdateValue {
                        old_check: old_value,
                        new: new_value,
                    },
                    test_key,
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

                Ok(GeneratedMessage::new(
                    CallMessage::UpdateValue {
                        old_check: wrong_old,
                        new: new_value,
                    },
                    test_key,
                    MessageOutcome::Reverted,
                ))
            }
        }
    }
}
