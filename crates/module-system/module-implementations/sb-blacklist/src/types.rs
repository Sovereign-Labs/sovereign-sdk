//! Types used by the `Blacklist` module.

use sov_modules_api::macros::serialize;
use sov_modules_api::Spec;

/// Genesis configuration for the blacklist module.
#[derive(Clone, Debug, PartialEq, Eq)]
#[serialize(Serde)]
#[serde(rename_all = "snake_case")]
pub struct BlacklistConfig<S: Spec> {
    /// Has authority for changing `manager` and toggling enforcement.
    pub owner: S::Address,

    /// Can set blacklist signers.
    pub manager: S::Address,

    /// Initial value for the global enforcement flag.
    /// When `true`, helper methods that respect enforcement will fail if
    /// blacklist checks are not satisfied.
    pub enforcement_enabled: bool,
}
