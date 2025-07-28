use schemars::JsonSchema;
use sov_modules_api::macros::serialize;
use sov_modules_api::{DaSpec, Spec};

use crate::PayeePolicy;

/// An event emitted by the paymaster module.
///
/// These events cover every change to the module state which
/// cannot be conveniently watched from the REST API. Changes to the
/// default policies for a payer and/or the list of allowed sequencer/updaters
/// are not currently emitted as events since those values can be directly observed from
/// the API by just querying the policy for a particular payer.
#[derive(Debug, PartialEq, Clone, JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "Event")]
pub enum Event<S: Spec> {
    /// A paymaster with the given address was registered.
    RegisteredPaymaster {
        #[allow(missing_docs)]
        address: S::Address,
    },
    /// The paymaster for the listed sequencer was set to the given value.
    SetPayerForSequencer {
        #[allow(missing_docs)]
        sequencer: <S::Da as DaSpec>::Address,
        #[allow(missing_docs)]
        payer: S::Address,
    },
    /// The paymaster for the listed sequencer was removed without being set to a new value.
    RemovedPayerForSequencer {
        #[allow(missing_docs)]
        sequencer: <S::Da as DaSpec>::Address,
        #[allow(missing_docs)]
        payer: S::Address,
    },
    /// A payer's policy for some particular payee was removed. The listed payee will now
    /// have its transactions handled according to the payer's default policy.
    RemovedPayeePolicy {
        #[allow(missing_docs)]
        payer: S::Address,
        #[allow(missing_docs)]
        payee: S::Address,
    },
    /// A particular payer added a policy override for some particular payee. This means that the listed payee
    /// will no longer have its transactions handled using the payer's default policy.
    AddedPayeePolicy {
        #[allow(missing_docs)]
        payer: S::Address,
        #[allow(missing_docs)]
        payee: S::Address,
        #[allow(missing_docs)]
        policy: PayeePolicy<S>,
    },
    /// The default policy for a payer was set to a new value.
    SetDefaultPayeePolicy {
        #[allow(missing_docs)]
        payer: S::Address,
        #[allow(missing_docs)]
        policy: PayeePolicy<S>,
    },
}
