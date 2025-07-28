use std::sync::Arc;

use derivative::Derivative;
use serde::{Deserialize, Serialize};
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::Spec;

use super::access_pattern::{
    AccessPatternChangeLogDiscriminant, AccessPatternChangeLogEntry, AccessPatternHarness,
    AccessPatternTag,
};
use super::bank::harness_interface::BankHarness;
use super::bank::{BankChangeLogDiscriminant, BankChangeLogEntry, BankTag};
use super::factory::CallMessageFactory;
use super::value_setter::{
    ValueSetterChangeLogDiscriminant, ValueSetterChangeLogEntry, ValueSetterHarness,
};
use crate::generators::synthetic_load::{
    SyntheticLoadChangeLogEntry, SyntheticLoadChangeLogEntryDiscriminant,
};
use crate::{ChangelogEntry, HarnessModule};

/// A basic call message generator factory that can be used with modules internal to the sovereign sdk
pub type BasicCallMessageFactory<S, RT, Acct = ()> =
    CallMessageFactory<S, RT, BasicTag, BasicChangeLogEntry<S>, Acct>;
/// A helper type that corresponds to bank modules compatible with the basic harness
pub type BasicBankHarness<S, RT, Acct = ()> =
    BankHarness<S, RT, BasicTag, BasicChangeLogEntry<S>, Acct>;
/// A helper type that corresponds to value setter modules compatible with the basic harness
pub type BasicValueSetterHarness<S, RT, Acct = ()> =
    ValueSetterHarness<S, RT, BasicTag, BasicChangeLogEntry<S>, Acct>;
/// A helper type that corresponds to access pattern modules compatible with the basic harness
pub type BasicAccessPatternHarness<S, RT, Acct = ()> =
    AccessPatternHarness<S, RT, BasicTag, BasicChangeLogEntry<S>, Acct>;
/// A helper type that contains a reference to a basic module
pub type BasicModuleRef<S, RT, Acct = ()> =
    Arc<dyn HarnessModule<S, RT, BasicTag, BasicChangeLogEntry<S>, Acct>>;

/// The set of tags supported by the [`BasicCallMessageFactory`].
#[derive(Clone, Copy, Derivative, Debug, derive_more::From)]
#[derivative(PartialEq, Eq, Hash)]
pub enum BasicTag {
    /// Tags for the bank module
    Bank(BankTag),
    /// Tags for the value setter module
    ValueSetter(()),
    /// Tags for the access pattern module
    AccessPattern(AccessPatternTag),
}

/// The set of change log entries supported by the [`BasicCallMessageFactory`].
#[derive(Clone, Debug, derive_more::From, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BasicChangeLogEntry<S: Spec> {
    /// Changes from the bank module
    Bank(BankChangeLogEntry<S>),
    /// Changes from the value setter module
    ValueSetter(ValueSetterChangeLogEntry),
    /// Changes from the synthetic load module
    SyntheticLoad(SyntheticLoadChangeLogEntry),
    /// Changes from the access pattern module
    AccessPattern(AccessPatternChangeLogEntry<S>),
}

/// Helper struct that can be used to discriminate between different [`BasicChangeLogEntry`]s.
#[derive(Debug, PartialEq, Eq, Derivative)]
#[derivative(Hash(bound = ""))]
pub enum BasicChangeLogDiscriminant<S: Spec> {
    /// Discriminants from the bank module
    Bank(BankChangeLogDiscriminant<S>),
    /// Discriminants from the value setter module
    ValueSetter(ValueSetterChangeLogDiscriminant),
    /// Discriminant from the synthetic load module
    SyntheticLoad(SyntheticLoadChangeLogEntryDiscriminant),
    /// Discriminants from the access pattern module
    AccessPattern(AccessPatternChangeLogDiscriminant),
}

#[async_trait]
impl<S: Spec> ChangelogEntry for BasicChangeLogEntry<S> {
    type ClientConfig = BasicClientConfig;

    type Discriminant = BasicChangeLogDiscriminant<S>;

    async fn assert_state(
        &self,
        rollup_state_accessor: Arc<Self::ClientConfig>,
    ) -> Result<(), anyhow::Error> {
        match self {
            BasicChangeLogEntry::Bank(b) => {
                b.assert_state(Arc::new((*rollup_state_accessor).clone().into()))
                    .await
            }
            BasicChangeLogEntry::ValueSetter(v) => {
                v.assert_state(Arc::new((*rollup_state_accessor).clone().into()))
                    .await
            }
            BasicChangeLogEntry::AccessPattern(v) => {
                v.assert_state(Arc::new((*rollup_state_accessor).clone().into()))
                    .await
            }
            BasicChangeLogEntry::SyntheticLoad(v) => {
                v.assert_state(Arc::new((*rollup_state_accessor).clone().into()))
                    .await
            }
        }
    }

    fn as_discriminant(&self) -> Self::Discriminant {
        match self {
            BasicChangeLogEntry::Bank(b) => BasicChangeLogDiscriminant::Bank(b.as_discriminant()),
            BasicChangeLogEntry::ValueSetter(v) => {
                BasicChangeLogDiscriminant::ValueSetter(v.as_discriminant())
            }
            BasicChangeLogEntry::AccessPattern(v) => {
                BasicChangeLogDiscriminant::AccessPattern(v.as_discriminant())
            }
            BasicChangeLogEntry::SyntheticLoad(v) => {
                BasicChangeLogDiscriminant::SyntheticLoad(v.as_discriminant())
            }
        }
    }
}

/// The basic configuration for any rollup http client.
#[derive(Debug, Clone)]
pub struct BasicClientConfig {
    /// The url to query.
    pub url: String,
    /// The rollup height to query, if necessary.
    pub rollup_height: Option<u64>,
}
