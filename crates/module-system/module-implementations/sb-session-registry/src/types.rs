//! Types used by the `SessionRegistry` module.

use schemars::JsonSchema;
use sov_modules_api::macros::serialize;
use sov_modules_api::Spec;

#[derive(Clone, Debug, PartialEq, Eq)]
#[serialize(Serde)]
#[serde(rename_all = "snake_case")]
pub struct RegistryConfig<S: Spec> {
    /// Has authority for changing `manager` and toggling enforcement.
    pub owner: S::Address,

    /// Can set session signers, and manage per-wallet bypass behavior.
    pub manager: S::Address,

    /// Initial value for the global enforcement flag.
    /// When `true`, helper methods that respect enforcement will fail if session checks are not satisfied
    pub enforcement_enabled: bool,

    /// Offset to extend all active session expiries by a fixed amount.
    /// Used in emergencies if backend services are down and need to extend sessions.
    pub expiry_offset: i64,
}

/// Per-wallet session state.
#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serialize(Borsh, Serde)]
pub struct Session {
    /// Session expiry timestamp (seconds since epoch, as provided by DA time).
    pub expiry_ts: i64,

    /// If `true`, this wallet bypasses normal session expiry checks.
    ///
    /// A bypassed wallet is treated as always having an active
    /// and present session.
    pub bypass: bool,
}
