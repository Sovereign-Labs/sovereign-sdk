//! Implements call message generation for the [`sov_value_setter::ValueSetter`] module.

use std::sync::Arc;

use http::HttpValueSetterClient;
use serde::{Deserialize, Serialize};
use sov_modules_api::prelude::arbitrary;
use sov_modules_api::prelude::arbitrary::Arbitrary;
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::{CryptoSpec, PrivateKey, Spec};
use sov_value_setter::{CallMessage, CallMessageDiscriminants};
use strum::EnumDiscriminants;

use crate::interface::{
    CallMessageGenerator, Distribution, GeneratedMessage, MessageValidity, Percent, Taggable,
};
use crate::state::{AccountState, ApplyToState};
use crate::{repeatedly, ChangelogEntry, MessageOutcome};

mod http;

mod harness_interface;

pub use harness_interface::*;

/// The state of a value setter account
#[derive(Debug, Clone)]
pub struct ValueSetterAccount<S: Spec> {
    pub(crate) private_key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
}

impl<S: Spec, Data> From<&AccountState<S, Data>> for ValueSetterAccount<S> {
    fn from(value: &AccountState<S, Data>) -> ValueSetterAccount<S> {
        ValueSetterAccount {
            private_key: value.private_key.clone(),
        }
    }
}

impl<S: Spec, Data> ApplyToState<S, Data> for ValueSetterAccount<S> {
    fn apply_to(self, _account: &mut AccountState<S, Data>) {}
}

impl<S: Spec> Taggable for ValueSetterAccount<S> {
    type Tag = ();

    fn add_tag(&mut self, _tag: Self::Tag) {}

    fn remove_tag(&mut self, _tag: Self::Tag) {}

    fn take_tags(&mut self) -> impl IntoIterator<Item = crate::interface::TagAction<Self::Tag>> {
        vec![].into_iter()
    }
}

/// A message generator for the `ValueSetter` module.
#[derive(Debug, Clone)]
pub struct ValueSetterGeneratorOptions {
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

/// A message generator for the `ValueSetter` module.
#[derive(Debug, Clone)]
pub struct ValueSetterMessageGenerator<S: Spec> {
    message_distribution: Distribution<CallMessageDiscriminants>,
    /// Configuration options controlling message generation parameters.
    options: ValueSetterGeneratorOptions,
    /// The private key of the admin of the value setter module
    admin_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
}

impl<S: Spec> ValueSetterMessageGenerator<S> {
    /// Creates a new [`ValueSetterMessageGenerator`]
    pub fn new(
        message_distribution: Distribution<CallMessageDiscriminants>,
        options: ValueSetterGeneratorOptions,
        admin_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    ) -> Self {
        Self {
            message_distribution,
            options,
            admin_key,
        }
    }
}

/// A complete description of any possible state change created by the [`ValueSetterMessageGenerator`].
#[derive(Debug, Clone, Deserialize, Serialize, EnumDiscriminants)]
#[serde(rename_all = "snake_case")]
#[strum_discriminants(name(ValueSetterChangeLogDiscriminant), derive(Hash))]
pub enum ValueSetterChangeLogEntry {
    /// The single value was updated
    ValueUpdated {
        /// The new value stored in state
        new_value: u32,
    },
    /// The vector of values was updated
    ManyValuesUpdated {
        /// The new value vector stored in state
        new_values: Vec<u8>,
    },
}

impl PartialEq<ValueSetterChangeLogEntry> for ValueSetterChangeLogEntry {
    fn eq(&self, other: &ValueSetterChangeLogEntry) -> bool {
        matches!(
            (self, other),
            (Self::ValueUpdated { .. }, Self::ValueUpdated { .. })
                | (
                    Self::ManyValuesUpdated { .. },
                    Self::ManyValuesUpdated { .. }
                )
        )
    }
}

#[async_trait]
impl ChangelogEntry for ValueSetterChangeLogEntry {
    type ClientConfig = HttpValueSetterClient;

    type Discriminant = ValueSetterChangeLogDiscriminant;

    async fn assert_state(
        &self,
        rollup_state_accessor: Arc<Self::ClientConfig>,
    ) -> Result<(), anyhow::Error> {
        match self {
            ValueSetterChangeLogEntry::ValueUpdated { new_value } => {
                let value = rollup_state_accessor.get_value().await;

                assert_eq!(value, Some(*new_value));
            }
            ValueSetterChangeLogEntry::ManyValuesUpdated { new_values } => {
                let values_len = rollup_state_accessor.get_many_values_len().await;

                assert_eq!(values_len, Some(new_values.len() as u64));

                for i in 0..values_len.unwrap() {
                    let value = rollup_state_accessor.get_many_values_item(i).await;
                    assert_eq!(value, Some(new_values[i as usize]));
                }
            }
        }

        Ok(())
    }

    fn as_discriminant(&self) -> Self::Discriminant {
        match self {
            ValueSetterChangeLogEntry::ValueUpdated { .. } => {
                ValueSetterChangeLogDiscriminant::ValueUpdated
            }
            ValueSetterChangeLogEntry::ManyValuesUpdated { .. } => {
                ValueSetterChangeLogDiscriminant::ManyValuesUpdated
            }
        }
    }
}

#[async_trait]
impl<S: Spec> CallMessageGenerator<S> for ValueSetterMessageGenerator<S> {
    type Module = sov_value_setter::ValueSetter<S>;

    type AccountView = ValueSetterAccount<S>;

    type ChangelogEntry = ValueSetterChangeLogEntry;

    type Tag = ();

    fn generate_setup_messages(
        &self,
        _u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
        _generator_state: &mut impl crate::interface::GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
    ) -> arbitrary::Result<
        Vec<
            crate::interface::GeneratedMessage<
                S,
                sov_value_setter::CallMessage<S>,
                Self::ChangelogEntry,
            >,
        >,
    > {
        Ok(vec![])
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
        crate::interface::GeneratedMessage<
            S,
            sov_value_setter::CallMessage<S>,
            Self::ChangelogEntry,
        >,
    > {
        match validity {
            MessageValidity::Valid => self.generate_valid_call_message(u),
            MessageValidity::Invalid => self.generate_invalid_call_message(u, generator_state),
        }
    }
}

impl<S: Spec> ValueSetterMessageGenerator<S> {
    /// We have two types of valid messages:
    /// 1. A message that sets a value and that is sent by an admin
    /// 2. A message that sets multiple values and that is sent by an admin
    pub fn generate_valid_call_message(
        &self,
        u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, ValueSetterChangeLogEntry>> {
        let message_type = self.message_distribution.select_value(u)?;

        match message_type {
            CallMessageDiscriminants::SetValue => {
                let value = u32::arbitrary(u)?;

                Ok(GeneratedMessage::new(
                    CallMessage::SetValue { value, gas: None },
                    self.admin_key.clone(),
                    MessageOutcome::Successful {
                        changes: vec![ValueSetterChangeLogEntry::ValueUpdated { new_value: value }],
                    },
                ))
            }
            CallMessageDiscriminants::SetValueAndSleep => {
                // Don't generate a sleep time, just set the value. We'd rather use real load than sleeps for soak testing.
                let value = u32::arbitrary(u)?;

                Ok(GeneratedMessage::new(
                    CallMessage::SetValue { value, gas: None },
                    self.admin_key.clone(),
                    MessageOutcome::Successful {
                        changes: vec![ValueSetterChangeLogEntry::ValueUpdated { new_value: value }],
                    },
                ))
            }
            CallMessageDiscriminants::SetManyValues => {
                let length = u.int_in_range(0..=self.options.maximum_vec_length)?;
                let mut values = Vec::with_capacity(length);

                for _ in 0..length {
                    values.push(u8::arbitrary(u)?);
                }

                Ok(GeneratedMessage::new(
                    CallMessage::SetManyValues(values.clone()),
                    self.admin_key.clone(),
                    MessageOutcome::Successful {
                        changes: vec![ValueSetterChangeLogEntry::ManyValuesUpdated {
                            new_values: values,
                        }],
                    },
                ))
            }
            CallMessageDiscriminants::AssertVisibleSlotNumber => {
                let value = u32::arbitrary(u)?;
                Ok(GeneratedMessage::new(
                    CallMessage::SetValue { value, gas: None },
                    self.admin_key.clone(),
                    MessageOutcome::Successful { changes: vec![] },
                ))
            }
            CallMessageDiscriminants::Panic => {
                unimplemented!("Panic is not supported in the transaction generator");
            }
        }
    }

    /// We have two types of invalid messages:
    /// 1. A message that sets a value and that is sent by a non-admin (or if the admin is not set)
    /// 2. A message that sets multiple values and that is sent by a non-admin (or if the admin is not set)
    ///
    /// This method should never fail.
    pub fn generate_invalid_call_message(
        &self,
        u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
        generator_state: &mut impl crate::interface::GeneratorState<
            S,
            AccountView = ValueSetterAccount<S>,
            Tag: From<()>,
        >,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, ValueSetterChangeLogEntry>> {
        let message_type = self.message_distribution.select_value(u)?;

        repeatedly!(
            let (_address, account) = generator_state.get_or_generate(Percent::fifty(), u)?;
            until: account.private_key.pub_key() != self.admin_key.pub_key(),
            on_failure: panic!("Impossible to get a non-admin account, when there should only be one admin!")
        );

        match message_type {
            CallMessageDiscriminants::AssertVisibleSlotNumber => {
                let message = CallMessage::AssertVisibleSlotNumber {
                    expected_visible_slot_number: u64::MAX,
                };
                Ok(GeneratedMessage {
                    message,
                    sender: account.private_key,
                    outcome: MessageOutcome::Reverted,
                })
            }
            CallMessageDiscriminants::Panic => {
                unimplemented!("Panic is not supported in the transaction generator");
            }
            _ => {
                let value = u32::arbitrary(u)?;
                let message = CallMessage::SetValue { value, gas: None };
                Ok(GeneratedMessage {
                    message,
                    sender: account.private_key,
                    outcome: MessageOutcome::Reverted,
                })
            }
        }
    }
}
