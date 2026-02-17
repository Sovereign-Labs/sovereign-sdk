//! Gas billing trait for gas payer layer settlement.
//!
//! This module defines the [`GasBiller`] trait which provides an interface for
//! gas billing operations. This trait is implemented by the Bank module to enable
//! gas payer layer billing in [`LayeredRevertableTxState`].
//!
//! [`LayeredRevertableTxState`]: crate::state::accessors::scratchpad::LayeredRevertableTxState

use crate::{Amount, Spec, StateAccessor};

/// Error type for gas billing operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GasBillingError {
    /// The gas payer does not have an account for the gas token.
    AccountDoesNotExist {
        /// String representation of the account.
        account: String,
    },
    /// Insufficient balance to pay for gas.
    InsufficientBalance {
        /// Required amount.
        required: Amount,
        /// Available balance.
        available: Amount,
    },
    /// State access error during billing.
    StateAccessError(String),
    /// Token transfer error during billing.
    TransferError(String),
}

impl std::fmt::Display for GasBillingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccountDoesNotExist { account } => {
                write!(f, "Gas payer account does not exist: {}", account)
            }
            Self::InsufficientBalance { required, available } => {
                write!(
                    f,
                    "Insufficient balance for gas: required {}, available {}",
                    required, available
                )
            }
            Self::StateAccessError(msg) => write!(f, "State access error: {}", msg),
            Self::TransferError(msg) => write!(f, "Transfer error: {}", msg),
        }
    }
}

impl std::error::Error for GasBillingError {}

/// Trait for gas billing operations.
///
/// This trait is implemented by modules that can provide gas token balance
/// information and perform gas token transfers. The primary implementor is
/// the Bank module.
///
/// # Use Case
/// When a gas payer layer is created in [`LayeredRevertableTxState`], the
/// gas payer's balance needs to be read to set up the gas meter. When the
/// layer is settled (committed or reverted), the gas consumed needs to be
/// billed to the gas payer.
///
/// # Example (Solidity Analogy)
/// Think of this like switching `msg.sender` context for gas accounting
/// in nested contract calls. The inner call's gas is charged to the callee's
/// account, not the original caller.
///
/// [`LayeredRevertableTxState`]: crate::state::accessors::scratchpad::LayeredRevertableTxState
pub trait GasBiller<S: Spec> {
    /// Get the gas token balance of an address.
    ///
    /// # Arguments
    /// * `address` - The address to query the balance for.
    /// * `state` - State accessor for reading balance.
    ///
    /// # Returns
    /// The gas token balance, or `None` if the account doesn't exist.
    fn gas_balance_of(
        &self,
        address: &S::Address,
        state: &mut impl StateAccessor,
    ) -> Result<Option<Amount>, GasBillingError>;

    /// Transfer gas tokens from one address to another.
    ///
    /// This is used to bill gas payers when settling a gas payer layer.
    /// The transfer is performed directly to ensure gas payments are permanent
    /// and not affected by layer reverts.
    ///
    /// # Arguments
    /// * `from` - The address to transfer from (gas payer).
    /// * `to` - The address to transfer to (typically sequencer/operator).
    /// * `amount` - The amount of gas tokens to transfer.
    /// * `state` - State accessor for performing the transfer.
    fn transfer_gas_tokens(
        &self,
        from: &S::Address,
        to: &S::Address,
        amount: Amount,
        state: &mut impl StateAccessor,
    ) -> Result<(), GasBillingError>;
}
