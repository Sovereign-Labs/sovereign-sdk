mod errors;
pub use errors::*;
use sov_rollup_interface::BasicAddress;
use sov_state::User;

use crate::{Amount, GetGasPrice, StateAccessor, StateWriter, TxState};

/// A trait that abstracts the generic logic for staking and un-staking across various sov-modules.
pub trait StakeRegistration {
    /// The primary address, which can be either the DA address or the Rollup address, depending on the use case.
    type PrimaryAddress: BasicAddress;
    /// Address on the rollup.
    type RollupAddress: BasicAddress;
    /// Custom module error type. This allows handling module specific errors in the library logic.
    type CustomError;
    /// The associated spec type.
    type Spec: crate::Spec;

    /// Tries to register a staker by staking the provided amount.
    ///
    /// # Errors
    /// Returns an error:
    ///  * If the staker is already registered
    ///  * If the staker does not have the funds to cover the bond
    ///  * If the state operations fail (i.e. if the state accessor is failible)
    #[allow(clippy::type_complexity)]
    fn register_staker<ST: TxState<Self::Spec>>(
        &mut self,
        primary_address: &Self::PrimaryAddress,
        rollup_address: &Self::RollupAddress,
        amount: Amount,
        state: &mut ST,
    ) -> Result<
        (),
        RegistrationError<
            Self::RollupAddress,
            Self::PrimaryAddress,
            <ST as StateWriter<User>>::Error,
            Self::CustomError,
        >,
    > {
        if self.get_allowed_staker(primary_address, state)?.is_some() {
            tracing::error!(staker = ?primary_address, "Staker already registered");
            return Err(RegistrationError::AlreadyRegistered(rollup_address.clone()));
        }

        self.transfer_bond_from_staker(rollup_address, amount, state)
            .map_err(|e| {
                tracing::error!(staker = ?primary_address, error = ?e, "Insufficient funds to register");
                RegistrationError::InsufficientFundsToRegister {
                    address: rollup_address.clone(),
                    amount,
                }
            })?;

        self.set_allowed_staker(primary_address, rollup_address, amount, state)?;
        tracing::trace!(%primary_address, %rollup_address, %amount, "Staker has been registered");

        Ok(())
    }

    /// Increases the balance of the sender, updating the state of the registry.
    ///
    /// # Errors
    /// Returns an error:
    ///  * If the staker is not registered
    ///  * If the staker does not have the funds to cover the increase
    ///  * If the state operations fail (i.e. if the state accessor is failible)
    #[allow(clippy::type_complexity)]
    fn deposit_funds<ST: TxState<Self::Spec>>(
        &mut self,
        staker: &Self::PrimaryAddress,
        amount: Amount,
        state: &mut ST,
    ) -> Result<
        (),
        RegistrationError<
            Self::RollupAddress,
            Self::PrimaryAddress,
            <ST as StateWriter<User>>::Error,
            Self::CustomError,
        >,
    > {
        let (address, balance) = self.get_allowed_staker(staker, state)?.ok_or_else(|| {
            tracing::error!("Staker not registered");
            RegistrationError::IsNotRegistered(staker.clone())
        })?;

        let balance =  balance.checked_add(amount).ok_or_else(|| {
                tracing::error!(staker = ?staker, amount = ?amount, balance = ?balance, "Topping account makes balance overflow");
                RegistrationError::ToppingAccountMakesBalanceOverflow {
                    address: address.clone(),
                    existing_balance: balance,
                    amount_to_add: amount,
                }
        })?;

        self.transfer_bond_from_staker(&address, amount, state)
            .map_err(|e| {
                tracing::error!(staker = ?staker, error = ?e, "Insufficient funds to top up account");
                RegistrationError::InsufficientFundsToTopUpAccount {
                    address: address.clone(),
                    amount_to_add: amount,
                }
            })?;

        self.set_allowed_staker(staker, &address, balance, state)?;

        Ok(())
    }

    /// Tries to unstake the sender.
    ///
    /// # Errors
    /// Returns an error:
    ///  * If the staker is not registered
    ///  * If the state operations fail (i.e. if the state accessor is failible)
    ///
    ///  Additionally, can error if the module does not have the funds to refund the bond, which
    ///  indicates a bug in the module.
    #[allow(clippy::type_complexity)]
    fn exit_staker<ST: TxState<Self::Spec>>(
        &mut self,
        staker: &Self::PrimaryAddress,
        state: &mut ST,
    ) -> Result<
        Amount,
        RegistrationError<
            Self::RollupAddress,
            Self::PrimaryAddress,
            <ST as StateWriter<User>>::Error,
            Self::CustomError,
        >,
    > {
        let (address, balance) = self.get_allowed_staker(staker, state)?.ok_or_else(|| {
            tracing::error!(staker = ?staker, "Staker not registered");
            RegistrationError::IsNotRegistered(staker.clone())
        })?;

        self.transfer_bond_to_staker(&address, balance, state)
            .map_err(|e| {
                tracing::error!(staker = ?staker, error = ?e, "Insufficient funds to refund stake");
                RegistrationError::InsufficientFundsToRefundStakedAmount {
                    address: address.clone(),
                    amount: balance,
                }
            })?;

        self.delete_allowed_staker(staker, state)?;

        Ok(balance)
    }

    /// The minimum allowed bond.
    fn get_minimum_bond<ST: TxState<Self::Spec> + GetGasPrice<Spec = Self::Spec>>(
        &self,
        state: &mut ST,
    ) -> Result<Option<Amount>, <ST as StateWriter<User>>::Error>;

    /// Get the allowed staker.
    #[allow(clippy::type_complexity)]
    fn get_allowed_staker<ST: TxState<Self::Spec>>(
        &self,
        address: &Self::PrimaryAddress,
        state: &mut ST,
    ) -> Result<Option<(Self::RollupAddress, Amount)>, <ST as StateWriter<User>>::Error>;

    /// Set the allowed staker.
    fn set_allowed_staker<ST: TxState<Self::Spec>>(
        &mut self,
        primary_address: &Self::PrimaryAddress,
        rollup_address: &Self::RollupAddress,
        amount: Amount,
        state: &mut ST,
    ) -> Result<(), <ST as StateWriter<User>>::Error>;

    /// Transfer bond from a staker to the rollup.
    fn transfer_bond_from_staker<ST: StateAccessor>(
        &mut self,
        address: &Self::RollupAddress,
        amount: Amount,
        state: &mut ST,
    ) -> anyhow::Result<()>;

    /// Transfer bond from the rollup to a staker.
    fn transfer_bond_to_staker<ST: StateAccessor>(
        &mut self,
        address: &Self::RollupAddress,
        amount: Amount,
        state: &mut ST,
    ) -> anyhow::Result<()>;

    /// Delete the allowed staker.
    fn delete_allowed_staker<ST: StateAccessor>(
        &mut self,
        address: &Self::PrimaryAddress,
        state: &mut ST,
    ) -> Result<(), <ST as StateWriter<User>>::Error>;
}
