//! Implements call message generation for the [`sov_synthetic_load::SyntheticLoad`]  module.

mod harness_interface;

use std::sync::Arc;

pub use harness_interface::*;
use serde::{Deserialize, Serialize};
use sov_modules_api::prelude::arbitrary;
use sov_modules_api::prelude::arbitrary::Arbitrary;
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::{CryptoSpec, Spec};
use sov_synthetic_load::{CallMessage, CallMessageDiscriminants};
use strum::{EnumDiscriminants, VariantArray};

use crate::generators::basic::BasicClientConfig;
use crate::interface::{
    CallMessageGenerator, Distribution, GeneratedMessage, MessageValidity, Percent, Taggable,
};
use crate::state::{AccountState, ApplyToState};
use crate::{ChangelogEntry, MessageOutcome};

/// The call message discriminants used by the [`sov_synthetic_load::SyntheticLoad`] module
pub const MESSAGES: &[sov_synthetic_load::CallMessageDiscriminants] =
    sov_synthetic_load::CallMessageDiscriminants::VARIANTS;

/// The state of a value setter account
#[derive(Debug, Clone)]
pub struct SyntheticLoadAccount<S: Spec> {
    pub(crate) private_key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
}

impl<S: Spec, Data> From<&'_ AccountState<S, Data>> for SyntheticLoadAccount<S> {
    fn from(value: &AccountState<S, Data>) -> SyntheticLoadAccount<S> {
        SyntheticLoadAccount {
            private_key: value.private_key.clone(),
        }
    }
}

impl<S: Spec, Data> ApplyToState<S, Data> for SyntheticLoadAccount<S> {
    fn apply_to(self, _account: &mut AccountState<S, Data>) {}
}

impl<S: Spec> Taggable for SyntheticLoadAccount<S> {
    type Tag = ();

    fn take_tags(&mut self) -> impl IntoIterator<Item = crate::interface::TagAction<Self::Tag>> {
        vec![].into_iter()
    }

    fn add_tag(&mut self, _tag: Self::Tag) {}

    fn remove_tag(&mut self, _tag: Self::Tag) {}
}

/// A message generator for the `ValueSetter` module.
#[derive(Debug, Clone)]
pub struct SyntheticLoadGeneratorOptions {
    /// The maximum length of a `SetManyValues` message
    pub maximum_vec_length: usize,
    /// The min and maximum number of operations for a `ReadAndSetManyIndividualValues` message
    pub min_and_max_number_of_individual_state_operations: (u64, u64),
    /// The min and maximum number of new values for a `ReadAndSetHeavyState` message
    pub min_and_max_number_of_new_values_for_heavy_state: (u64, u64),
    /// The min and maximum number of iterations for a `RunCPUHeavyOperation` message
    pub min_and_max_number_of_iterations_for_cpu_heavy_operation: (u64, u64),
    /// Max heavy state size
    pub max_heavy_state_size: u64,
}

/// A message generator for the [`sov_synthetic_load::SyntheticLoad`] module.
#[derive(Debug, Clone)]
pub struct SyntheticLoadMessageGenerator<S: Spec> {
    message_distribution: Distribution<CallMessageDiscriminants>,
    /// Configuration options controlling message generation parameters.
    options: SyntheticLoadGeneratorOptions,
    caller_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
}

impl<S: Spec> SyntheticLoadMessageGenerator<S> {
    /// Creates a new [`SyntheticLoadMessageGenerator`]
    pub fn new(
        message_distribution: Distribution<CallMessageDiscriminants>,
        options: SyntheticLoadGeneratorOptions,
        caller_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    ) -> Self {
        Self {
            message_distribution,
            options,
            caller_key,
        }
    }
}

/// A complete description of any possible state change created by the [`SyntheticLoadMessageGenerator`].
#[derive(Debug, Clone, Deserialize, Serialize, EnumDiscriminants)]
#[serde(rename_all = "snake_case")]
#[strum_discriminants(name(SyntheticLoadChangeLogEntryDiscriminant), derive(Hash))]
pub enum SyntheticLoadChangeLogEntry {
    /// The vector of values was updated
    ManyValuesUpdated {
        /// The new value vector stored in state
        new_values: Vec<u8>,
    },
}

impl PartialEq<SyntheticLoadChangeLogEntry> for SyntheticLoadChangeLogEntry {
    fn eq(&self, other: &SyntheticLoadChangeLogEntry) -> bool {
        matches!(
            (self, other),
            (
                Self::ManyValuesUpdated { .. },
                Self::ManyValuesUpdated { .. }
            )
        )
    }
}

#[allow(missing_docs)]
pub struct SyntheticLoadClientConfig {}

impl From<BasicClientConfig> for SyntheticLoadClientConfig {
    fn from(_config: BasicClientConfig) -> Self {
        Self {}
    }
}

#[async_trait]
impl ChangelogEntry for SyntheticLoadChangeLogEntry {
    type ClientConfig = SyntheticLoadClientConfig;

    type Discriminant = SyntheticLoadChangeLogEntryDiscriminant;

    async fn assert_state(
        &self,
        _rollup_state_accessor: Arc<Self::ClientConfig>,
    ) -> Result<(), anyhow::Error> {
        // TODO: hm?
        Ok(())
    }

    fn as_discriminant(&self) -> Self::Discriminant {
        match self {
            SyntheticLoadChangeLogEntry::ManyValuesUpdated { .. } => {
                SyntheticLoadChangeLogEntryDiscriminant::ManyValuesUpdated
            }
        }
    }
}

#[async_trait]
impl<S: Spec> CallMessageGenerator<S> for SyntheticLoadMessageGenerator<S> {
    type Module = sov_synthetic_load::SyntheticLoad<S>;

    type Tag = ();

    type AccountView = SyntheticLoadAccount<S>;

    type ChangelogEntry = SyntheticLoadChangeLogEntry;

    fn generate_setup_messages(
        &self,
        _u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
        _generator_state: &mut impl crate::interface::GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
    ) -> arbitrary::Result<Vec<GeneratedMessage<S, CallMessage, Self::ChangelogEntry>>> {
        Ok(Vec::new())
    }

    fn generate_call_message(
        &self,
        u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
        generator_state: &mut impl crate::interface::GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
        validity: crate::interface::MessageValidity,
    ) -> sov_modules_api::prelude::arbitrary::Result<
        crate::interface::GeneratedMessage<S, CallMessage, Self::ChangelogEntry>,
    > {
        match validity {
            MessageValidity::Valid => self.generate_valid_call_message(u),
            MessageValidity::Invalid => self.generate_invalid_call_message(u, generator_state),
        }
    }
}

impl<S: Spec> SyntheticLoadMessageGenerator<S> {
    /// We have these types of valid messages:
    /// 1. ReadAndSetManyIndividualValues
    /// 2. ReadAndSetHeavyState
    /// 3. RunCPUHeavyOperation
    pub fn generate_valid_call_message(
        &self,
        u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage, SyntheticLoadChangeLogEntry>> {
        let message_type = self.message_distribution.select_value(u)?;

        match message_type {
            CallMessageDiscriminants::ReadAndSetManyIndividualValues => {
                let (min_number_of_operations, max_number_of_operations) = self
                    .options
                    .min_and_max_number_of_individual_state_operations;
                let number_of_operations =
                    u.int_in_range(min_number_of_operations..=max_number_of_operations)?;

                Ok(GeneratedMessage::new(
                    CallMessage::ReadAndSetManyIndividualValues {
                        number_of_operations,
                        salt: u64::arbitrary(u)?,
                    },
                    self.caller_key.clone(),
                    MessageOutcome::Successful { changes: vec![] },
                ))
            }
            CallMessageDiscriminants::ReadAndSetHeavyState => {
                let (min_number_of_new_values, max_number_of_new_values) = self
                    .options
                    .min_and_max_number_of_new_values_for_heavy_state;
                let number_of_new_values =
                    u.int_in_range(min_number_of_new_values..=max_number_of_new_values)?;

                Ok(GeneratedMessage::new(
                    CallMessage::ReadAndSetHeavyState {
                        number_of_new_values,
                        max_heavy_state_size: self.options.max_heavy_state_size,
                        salt: u64::arbitrary(u)?,
                    },
                    self.caller_key.clone(),
                    MessageOutcome::Successful { changes: vec![] },
                ))
            }
            CallMessageDiscriminants::RunCPUHeavyOperation => {
                let (min_number_of_iterations, max_number_of_iterations) = self
                    .options
                    .min_and_max_number_of_iterations_for_cpu_heavy_operation;
                let iterations =
                    u.int_in_range(min_number_of_iterations..=max_number_of_iterations)?;

                Ok(GeneratedMessage::new(
                    CallMessage::RunCPUHeavyOperation { iterations },
                    self.caller_key.clone(),
                    MessageOutcome::Successful { changes: vec![] },
                ))
            }
        }
    }

    /// We have 1 type of invalid messages:
    /// This method should never fail.
    pub fn generate_invalid_call_message(
        &self,
        u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
        generator_state: &mut impl crate::interface::GeneratorState<
            S,
            AccountView = SyntheticLoadAccount<S>,
            Tag: From<()>,
        >,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage, SyntheticLoadChangeLogEntry>> {
        let (_, account) = generator_state.get_or_generate(Percent::fifty(), u)?;

        {
            let message = CallMessage::RunCPUHeavyOperation {
                iterations: u64::MAX,
            };
            Ok(GeneratedMessage {
                message,
                sender: account.private_key,
                outcome: MessageOutcome::Reverted,
            })
        }
    }
}
