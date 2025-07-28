use core::result::Result::Ok;
use std::fmt::Debug;

use derivative::Derivative;
use sov_modules_api::macros::{serialize, UniversalWallet};
use thiserror::Error;
use tracing::error;

use crate::Amount;

/// This enumeration represents the available call messages for interacting with the `AttesterIncentives` module.
#[derive(Derivative, Clone, PartialEq, Eq, schemars::JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
pub enum CallMessage {
    /// Register an attester, the parameter is the bond amount
    RegisterAttester(Amount),
    /// Start the first phase of the two-phase exit process
    BeginExitAttester,
    /// Finish the two phase exit
    ExitAttester,
    /// Register a challenger, the parameter is the bond amount
    RegisterChallenger(Amount),
    /// Exit a challenger
    ExitChallenger,
    /// Increases the balance of the attester.    
    DepositAttester(Amount),
}

// Manually implement Debug to remove spurious Debug bound on S::Storage
impl Debug for CallMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RegisterAttester(arg0) => f.debug_tuple("RegisterAttester").field(arg0).finish(),
            Self::DepositAttester(arg0) => f.debug_tuple("DepositAttester").field(arg0).finish(),
            Self::BeginExitAttester => write!(f, "BeginExitAttester"),
            Self::ExitAttester => write!(f, "ExitAttester"),
            Self::RegisterChallenger(arg0) => {
                f.debug_tuple("RegisterChallenger").field(arg0).finish()
            }
            Self::ExitChallenger => write!(f, "ExitChallenger"),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq, Clone, Copy, schemars::JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
/// Error type that explains why a user is slashed
pub enum SlashingReason {
    #[error("Transition isn't found")]
    /// The specified transition does not exist
    TransitionNotFound,

    #[error("The attestation does not contain the right block hash and post-state transition")]
    /// The specified transition is invalid (block hash, post-root hash or validity condition)
    TransitionInvalid,

    #[error("The initial hash of the transition is invalid")]
    /// The initial hash of the transition is invalid.
    InvalidInitialHash,

    #[error("The zk proof is invalid")]
    /// The zk proof is invalid
    InvalidZkProof,

    #[error("No invalid transition to challenge")]
    /// No invalid transition to challenge.
    NoInvalidTransition,
}
