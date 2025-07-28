use sov_bank::Amount;
use sov_modules_api::macros::serialize;
use sov_modules_api::Spec;

use crate::SlashingReason;

/// Events for attester incentives
#[derive(Debug, PartialEq, Clone, schemars::JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "Event")]
pub enum Event<S: Spec> {
    /// Event for User Slashed
    UserSlashed {
        /// The address of the user who was slashed.
        address: S::Address,
        /// The reason the user was slashed.
        reason: SlashingReason,
    },
    /// Event for registration of a new attester.
    RegisteredAttester {
        /// The amount of tokens deposited by this call.
        amount: Amount,
    },

    /// Event for registration of a new challenger.
    RegisteredChallenger {
        /// The amount of tokens deposited by this call.
        amount: Amount,
    },

    /// Event for exiting an attester.
    ExitedAttester {
        /// The number of tokens returned to the caller's bank balance.
        amount_withdrawn: Amount,
    },
    /// Event for a new deposit.
    BondedChallenger {
        /// The amount of tokens deposited by this call.
        new_deposit: Amount,
        /// The total bond of the challenger after this call.
        total_bond: Amount,
    },
    /// Event for a new deposit
    NewDeposit {
        /// The amount of tokens deposited by this call.
        new_deposit: Amount,
        /// The total bond of the challenger after this call.
        total_bond: Amount,
    },
    /// Event for exiting a challenger.
    ExitedChallenger {
        /// The number of tokens returned to the caller's bank balance.
        amount_withdrawn: Amount,
    },
}
