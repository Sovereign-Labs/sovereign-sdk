use schemars::JsonSchema;
use sov_modules_api::macros::serialize;
use sov_modules_api::Spec;

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "Event")]
pub enum Event<S: Spec> {
    ManagerSet {
        old_manager: Option<S::Address>,
        new_manager: S::Address,
    },

    EnforcementEnabledSet {
        enabled: bool,
    },

    SessionSignerSet {
        signer: S::Address,
        allowed: bool,
    },

    SessionSet {
        wallet: S::Address,
        expiry_ts: i64,
    },

    BypassSet {
        wallet: S::Address,
        bypass: bool,
    },

    ExpiryOffsetUpdated {
        old_offset: Option<i64>,
        new_offset: i64,
    },
}
