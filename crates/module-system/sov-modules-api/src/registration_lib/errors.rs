use sov_rollup_interface::BasicAddress;
use thiserror::Error;

use crate::Amount;

/// Errors that can be raised by the `Registry` library.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistrationError<
    RollupAddress: BasicAddress,
    NotRegAddress: BasicAddress,
    AccessorError,
    CustomError,
> {
    #[error("The provided address is not registered")]
    /// The provided address is not registered.
    IsNotRegistered(NotRegAddress),

    #[error("Insufficient funds on the sender's account to top up its staked balance")]
    /// Insufficient funds on the sender's account to top up its staked balance
    InsufficientFundsToTopUpAccount {
        /// The address of the sender's account.
        address: RollupAddress,
        /// The amount to add to the balance of the sender's account.
        amount_to_add: Amount,
    },

    #[error("Stake amount={bond_amount} below the minimum={minimum_bond_amount}")]
    /// Stake amount below the minimum needed.
    InsufficientStakeAmount {
        /// The address of the sender's account.
        address: RollupAddress,
        /// The amount of gas tokens the sender is trying to stake.
        bond_amount: Amount,
        /// The minimum amount of gas tokens to stake.
        minimum_bond_amount: Amount,
    },

    #[error("The provided amount makes the balance of the beneficiary's account overflow.")]
    /// The provided amount makes the balance of the beneficiary's account overflow.
    ToppingAccountMakesBalanceOverflow {
        /// The address of the beneficiary's account.
        address: RollupAddress,
        /// The existing staked balance of the beneficiary's account.
        existing_balance: Amount,
        /// The amount to add to the balance of the beneficiary's account.
        amount_to_add: Amount,
    },

    #[error(
        "The module account does not have enough funds to refund staked amount. This is a bug"
    )]
    /// The module account does not have enough funds to refund the staked amount.
    InsufficientFundsToRefundStakedAmount {
        /// The address of the sender's account.
        address: RollupAddress,
        /// The amount of gas tokens to refund
        amount: Amount,
    },

    #[error(
        "The minimum bond is not set. This is a bug - the minimum bond should be set at genesis"
    )]
    /// The minimum bond is not set. This is a bug - the minimum bond should be set at genesis
    NoMinimumBondSet,

    #[error("The user is already registered")]
    /// The user is already registered.
    AlreadyRegistered(RollupAddress),

    #[error("The user has already initiated a withdrawal")]
    /// The user has already initiated a withdrawal.
    WithdrawalAlreadyPending(RollupAddress),

    #[error("The sender's account does not have enough funds to register itself")]
    /// The sender's account does not have enough funds to register itself.
    InsufficientFundsToRegister {
        /// The address of the sender's account.
        address: RollupAddress,
        /// The amount of gas tokens to stake
        amount: Amount,
    },

    /// An error occurred when accessing the state
    #[error("An error occurred when accessing the state, error: {0}")]
    StateAccessorError(#[from] AccessorError),

    /// Custom error.
    #[error("Custom error: {0}")]
    Custom(CustomError),
}
