use sov_modules_api::transaction::AuthenticatedTransactionData;
use sov_modules_api::{Gas, Spec, StateAccessor};
use thiserror::Error;

use crate::utils::IntoPayable;
use crate::{config_gas_token_id, Bank, Coins};

/// Error types that can be raised by the `reserve_gas` method
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ReserveGasError {
    #[error("The payer {account} does not have an account in the `Bank` module for the gas token")]
    /// The payer does not have an account in the `Bank` module for the gas token
    AccountDoesNotExist {
        /// String representation of rollup address
        account: String,
    },
    #[error("Insufficient balance to pay for the transaction gas")]
    /// The sender balance is not high enough to pay for the gas.
    InsufficientBalanceToReserveGas,
    #[error("The current gas price is too high to cover the maximum fee for the transaction")]
    /// The current gas price is too high to cover the maximum fee for the transaction.
    CurrentGasPriceTooHigh,
    #[error("The transaction's gas limit is too high for this payer")]
    /// The transaction's gas limit is too high for this payer.
    MaxGasLimitExceeded,
    /// Impossible to transfer the gas from the payer to the bank
    #[error("Impossible to transfer the gas from the payer to the bank")]
    ImpossibleToTransferGas(String),
    /// Error occurred while accessing the state
    #[error("An error occurred while accessing the state: {0}")]
    StateAccessError(String),
}

impl<S: Spec> Bank<S> {
    /// Reserve the gas necessary to execute a transaction. The gas is locked at the bank's address
    /// This method loosely follows the-EIP 1559 gas price calculation.
    #[allow(clippy::result_large_err)]
    pub fn reserve_gas(
        &mut self,
        tx: &AuthenticatedTransactionData<S>,
        gas_price: &<S::Gas as Gas>::Price,
        payer: &S::Address,
        state: &mut impl StateAccessor,
    ) -> Result<(), ReserveGasError> {
        // We need to do the explicit check (outside of a closure) because otherwise `state_checkpoint` would be captured.
        let balance = match self
            .get_balance_of(&payer.clone(), config_gas_token_id(), state)
            .map_err(|e| ReserveGasError::StateAccessError(e.to_string()))?
        {
            Some(balance) => balance,
            None => {
                return Err(ReserveGasError::AccountDoesNotExist {
                    account: payer.to_string(),
                })
            }
        };

        // the signer must be able to afford the transaction
        if balance < tx.0.max_fee {
            return Err(ReserveGasError::InsufficientBalanceToReserveGas);
        }

        if tx.0.max_fee == 0 {
            tracing::warn!(
                %payer,
                "Trying to reserve gas for tx with zero max fee"
            );
        }

        if let Some(gas_limit) = &tx.0.gas_limit {
            let gas_value = gas_limit
                .checked_value(gas_price)
                .ok_or(ReserveGasError::CurrentGasPriceTooHigh)?;

            // We need to check the gas price in case the user has provided a gas limit.
            if tx.0.max_fee < gas_value.0 {
                return Err(ReserveGasError::CurrentGasPriceTooHigh);
            }
        }

        // We lock the `max_fee` amount into the `Bank` module.
        // We actually **need** to do that transfer because the payer account balance may change during the execution of the transaction.
        // Only do this after all checks have passed because the paymaster does not revert on error, so
        // any state changes may persist!
        let id = self.id;
        if let Err(err) = self.transfer_from(
            payer,
            id.to_payable(),
            Coins {
                amount: tx.0.max_fee,
                token_id: config_gas_token_id(),
            },
            state,
        ) {
            return Err(ReserveGasError::ImpossibleToTransferGas(err.to_string()));
        }

        Ok(())
    }
}
