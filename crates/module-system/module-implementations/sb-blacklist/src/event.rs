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

    BlacklistSignerSet {
        signer: S::Address,
        allowed: bool,
    },

    BlacklistSet {
        wallet: S::Address,
        blacklisted: bool,
    },

    OwnershipTransferred {
        old_owner: S::Address,
        new_owner: S::Address,
    },
}
