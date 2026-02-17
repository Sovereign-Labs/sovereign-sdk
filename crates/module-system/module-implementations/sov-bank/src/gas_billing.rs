//! Implementation of `GasBiller` trait for the Bank module.
//!
//! This enables the Bank to be used for gas payer layer billing in
//! [`LayeredRevertableTxState`].
//!
//! [`LayeredRevertableTxState`]: sov_modules_api::state::accessors::scratchpad::LayeredRevertableTxState

use sov_modules_api::{Amount, GasBiller, GasBillingError, Spec, StateAccessor};

use crate::{config_gas_token_id, Bank, Coins};

impl<S: Spec> GasBiller<S> for Bank<S> {
    fn gas_balance_of(
        &self,
        address: &S::Address,
        state: &mut impl StateAccessor,
    ) -> Result<Option<Amount>, GasBillingError> {
        self.get_balance_of(address, config_gas_token_id(), state)
            .map_err(|e| GasBillingError::StateAccessError(e.to_string()))
    }

    fn transfer_gas_tokens(
        &self,
        from: &S::Address,
        to: &S::Address,
        amount: Amount,
        state: &mut impl StateAccessor,
    ) -> Result<(), GasBillingError> {
        // Check if the payer has an account
        let balance = self
            .get_balance_of(from, config_gas_token_id(), state)
            .map_err(|e| GasBillingError::StateAccessError(e.to_string()))?;

        let balance = balance.ok_or_else(|| GasBillingError::AccountDoesNotExist {
            account: format!("{:?}", from),
        })?;

        // Check if the payer has sufficient balance
        if balance < amount {
            return Err(GasBillingError::InsufficientBalance {
                required: amount,
                available: balance,
            });
        }

        // Perform the transfer
        // Note: We clone self to get a mutable reference for transfer_from.
        // This is safe because transfer_from only modifies state, not Bank's internal fields.
        let mut bank_clone = self.clone();
        bank_clone
            .transfer_from(
                from,
                to,
                Coins {
                    amount,
                    token_id: config_gas_token_id(),
                },
                state,
            )
            .map_err(|e| GasBillingError::TransferError(e.to_string()))
    }
}

// Note: Tests for GasBiller implementation are in the integration tests
// with LayeredRevertableTxState where full state infrastructure is available.
