use anyhow::Result;
use core::fmt::Debug;

use crate::{Amount, Gas, GasArray, GasMeter, GasMeteringError, GetGasPrice, Spec};

/// The default Ethereum block gas limit: 1B
pub const ETHEREUM_BLOCK_GAS_LIMIT: u64 = 1_000_000_000;
/// The default Ethereum tx gas limit: 30M
pub const ETHEREUM_TX_GAS_LIMIT: u64 = 30_000_000;

/// A struct that keeps track of the gas used.
/// The gas meter continues running until it either depletes its funds or runs out of gas, depending on its configuration.
/// It also ensures that the gas used will not overflow when multiplied by the gas price.
#[derive(Clone, Copy, Debug)]
pub struct BasicGasMeter<S: Spec> {
    /// Amount of gas available at the moment of the gas meter initialization
    pub initial_gas: S::Gas,
    /// Amount of gas remaining
    pub remaining_gas: S::Gas,
    /// Amount of funds available
    pub remaining_funds: Option<Amount>,
    /// Gas price
    pub gas_price: <S::Gas as Gas>::Price,
}

/// Contain information about the gas usage of a gas.
pub struct GasInfo<GU: Gas> {
    /// The gas value.
    pub gas_value: Amount,
    /// The current gas used accumulated by the stake meter.
    pub gas_used: GU,
    /// The current gas price
    pub gas_price: GU::Price,
}

impl<S: Spec> BasicGasMeter<S> {
    /// Get gas info from the `BasicGasMeter`
    pub fn gas_info(&self) -> GasInfo<S::Gas> {
        let gas_used = self
            .initial_gas
            .checked_sub(self.remaining_gas)
            .expect("The remaining gas can't be greater than the initial gas");

        let gas_value = gas_used
            .checked_value(self.gas_price)
            // SAFETY: This is impossible because we check for oveflows in `BasicGasMeter::charge_gas_inner`.
            .expect("BasicGasMeter error. The gas value should be possible to compute");

        GasInfo {
            gas_value,
            gas_used,
            gas_price: self.gas_price,
        }
    }

    /// Creates a new `BasicGasMeter`.
    pub fn new_with_funds_and_gas(
        remaining_funds: Amount,
        remaining_gas: S::Gas,
        gas_price: <S::Gas as Gas>::Price,
    ) -> Self {
        Self {
            initial_gas: remaining_gas,
            remaining_gas,
            remaining_funds: Some(remaining_funds),
            gas_price,
        }
    }

    /// Creates a new `BasicGasMeter` for API access with gas limit set at ETH block gas limit.
    pub fn new_api(gas_price: <S::Gas as Gas>::Price) -> Self {
        let remaining_gas = [ETHEREUM_BLOCK_GAS_LIMIT, ETHEREUM_BLOCK_GAS_LIMIT].into();
        Self::new_with_funds_and_gas(Amount::MAX, remaining_gas, gas_price)
    }

    /// Creates a new `BasicGasMeter`
    pub fn new_with_gas(remaining_gas: S::Gas, gas_price: <S::Gas as Gas>::Price) -> Self {
        Self {
            initial_gas: remaining_gas,
            remaining_gas,
            remaining_funds: None,
            gas_price,
        }
    }

    fn compute_remaining_funds(
        &self,
        remaining_funds: Amount,
        amount: S::Gas,
    ) -> Result<Amount, GasMeteringError<S::Gas>> {
        let amount_value = amount.checked_value(self.gas_price).ok_or_else(|| {
            GasMeteringError::Overflow(
                "Charge Funds: Unable to charge gas, because the calculation overflows".to_string(),
            )
        })?;

        remaining_funds.checked_sub(amount_value).ok_or_else(|| {
            tracing::warn!(%remaining_funds, amount_to_charge = %amount_value, "Out of gas during `compute_remaining_funds`");
            GasMeteringError::OutOfFunds {
                amount_to_charge: amount_value,
                remaining_funds,
                gas_price: self.gas_price,
            }
        })
    }

    fn compute_remaining_gas(
        &self,
        remaining_gas: S::Gas,
        amount: S::Gas,
    ) -> Result<S::Gas, GasMeteringError<S::Gas>> {
        remaining_gas.checked_sub(amount).ok_or_else(|| {
            tracing::warn!(?remaining_gas, amount_to_charge = ?amount, "Out of gas during `compute_remaining_gas`");
            GasMeteringError::OutOfGas {
                gas_to_charge: amount,
                gas_price: self.gas_price,
                initial_gas: self.initial_gas,
                remaining_gas: self.remaining_gas,
            }
        })
    }

    fn charge_gas_inner(&mut self, amount: S::Gas) -> Result<(), GasMeteringError<S::Gas>> {
        let mut new_remaining_funds = None;

        if let Some(remaining_funds) = self.remaining_funds {
            new_remaining_funds = Some(self.compute_remaining_funds(remaining_funds, amount)?);
        }

        let new_remaining_gas = self.compute_remaining_gas(self.remaining_gas, amount)?;
        // Here we check that the current gas_used won't overflow when multiplied by the price.
        // This ensures that after execution, it is always safe to convert the total gas used to a token value.
        {
            let gas_used = self
                .initial_gas
                .checked_sub(new_remaining_gas)
                .expect("The remaining gas can't be greater than the initial gas");

            gas_used.checked_value(self.gas_price).ok_or_else(|| {
                GasMeteringError::Overflow(
                    "Charge Gas: Unable to charge gas, because the calculation overflows"
                        .to_string(),
                )
            })?;
        }

        self.remaining_funds = new_remaining_funds;
        self.remaining_gas = new_remaining_gas;

        Ok(())
    }
}

impl<S: Spec> GasMeter for BasicGasMeter<S> {
    type Spec = S;
    fn charge_gas(&mut self, amount: S::Gas) -> Result<(), GasMeteringError<S::Gas>> {
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%amount, gas_before = %self.remaining_gas, funds_before = ?self.remaining_funds, "Charging gas");
        self.charge_gas_inner(amount)?;

        #[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
        if let Some(name) = amount.name() {
            if sov_metrics::GAS_CONSTANTS
                .try_with(|var| {
                    let mut var = var.borrow_mut();

                    if let Some(const_count) = var.get_mut(name) {
                        *const_count = const_count.checked_add(1).unwrap();
                    } else {
                        var.insert(name.to_string(), 1);
                    }
                })
                .is_err()
            {
                tracing::trace!(
                    "Trying to gather gas constants without metrics collection enabled"
                );
            }
        }

        Ok(())
    }

    fn charge_linear_gas(
        &mut self,
        amount: S::Gas,
        parameter: u32,
    ) -> Result<(), GasMeteringError<<S as Spec>::Gas>> {
        let total_amount = amount
            .checked_scalar_product(parameter as u64)
            .ok_or_else(|| {
                GasMeteringError::Overflow(format!(
                    "Unable to charge gas. The product of {amount} to {parameter} is overflowing"
                ))
            })?;
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%total_amount, parameter, gas_before = %self.remaining_gas, funds_before = ?self.remaining_funds, "Charging linear gas");
        self.charge_gas_inner(total_amount)?;

        #[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
        if let Some(name) = amount.name() {
            if parameter > 0
                && sov_metrics::GAS_CONSTANTS
                    .try_with(|var| {
                        let param_i64 = parameter.into();
                        let mut var = var.borrow_mut();

                        if let Some(const_count) = var.get_mut(name) {
                            *const_count = const_count.checked_add(param_i64).unwrap();
                        } else {
                            var.insert(name.to_string(), param_i64);
                        }
                    })
                    .is_err()
            {
                tracing::trace!(
                    "Trying to gather gas constants without metrics collection enabled"
                );
            };
        }

        Ok(())
    }

    fn try_as_basic_gas_meter(&mut self) -> Option<&mut BasicGasMeter<Self::Spec>> {
        Some(self)
    }

    #[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
    fn remove_gas_pattern(&mut self, amount: &<Self::Spec as Spec>::Gas, parameter: u32) {
        if let Some(name) = amount.name() {
            if parameter > 0
                && sov_metrics::GAS_CONSTANTS
                    .try_with(|var| {
                        let param_i64 = parameter.into();
                        let mut var = var.borrow_mut();

                        if let Some(const_count) = var.get_mut(name) {
                            *const_count = const_count.checked_sub(param_i64).unwrap();
                        } else {
                            var.insert(name.to_string(), -param_i64);
                        }
                    })
                    .is_err()
            {
                tracing::trace!(
                    "Trying to gather gas constants without metrics collection enabled"
                );
            };
        }
    }
}

impl<S: Spec> GetGasPrice for BasicGasMeter<S> {
    type Spec = S;
    fn gas_price(&self) -> <<Self::Spec as Spec>::Gas as Gas>::Price {
        self.gas_price
    }
}

#[cfg(test)]
mod tests {
    use sov_mock_da::MockDaSpec;
    use sov_mock_zkvm::MockZkvm;
    use sov_rollup_interface::execution_mode::Native;

    use crate::{default_spec::DefaultSpec, GasPrice, GasUnit};

    use super::*;

    type S = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    const STANDARD_GAS_PRICE: GasPrice<2> = GasPrice {
        value: [Amount::new(1), Amount::new(1)],
    };
    const MAX_GAS_PRICE: GasPrice<2> = GasPrice {
        value: [Amount::MAX, Amount::MAX],
    };

    fn assert_charge_succeeds(meter: &mut BasicGasMeter<S>, gas: GasUnit<2>) {
        assert!(
            meter.charge_gas(gas).is_ok(),
            "It should be possible to charge gas"
        );
    }

    fn assert_charge_fails(meter: &mut BasicGasMeter<S>, gas: GasUnit<2>) {
        assert!(
            meter.charge_gas(gas).is_err(),
            "The gas meter should not be able to charge gas if there is not enough gas reserved"
        );
    }

    fn assert_gas_used_equals(meter: &BasicGasMeter<S>, expected: GasUnit<2>) {
        assert_eq!(
            meter.gas_info().gas_used,
            expected,
            "The gas used should be the same as the gas charged"
        );
    }

    fn assert_gas_price_equals(meter: &BasicGasMeter<S>, expected: GasPrice<2>) {
        assert_eq!(meter.gas_info().gas_price, expected);
    }

    #[test]
    fn test_charge_gas_fails_with_zeroed_gas() {
        let mut gas_meter =
            BasicGasMeter::<S>::new_with_gas(GasUnit::<2>::ZEROED, STANDARD_GAS_PRICE);

        assert_charge_fails(&mut gas_meter, GasUnit::<2>::from([100; 2]));
    }

    #[test]
    fn test_charge_gas_fails_with_zero_gas() {
        let gas = GasUnit::<2>::from([0, 0]);
        let mut gas_meter = BasicGasMeter::<S>::new_with_gas(gas, STANDARD_GAS_PRICE);

        assert_charge_fails(&mut gas_meter, GasUnit::<2>::from([100; 2]));
    }

    #[test]
    fn test_charge_gas_fails_when_partial_gas_insufficient() {
        let gas = GasUnit::<2>::from([1000, 99]);
        let mut gas_meter = BasicGasMeter::<S>::new_with_gas(gas, STANDARD_GAS_PRICE);

        assert_charge_fails(&mut gas_meter, GasUnit::<2>::from([100; 2]));
    }

    #[test]
    fn test_charge_with_funds_tracks_usage() {
        const REMAINING_FUNDS: u64 = 100;

        let mut gas_meter = BasicGasMeter::<S>::new_with_funds_and_gas(
            Amount::from(REMAINING_FUNDS),
            GasUnit::<2>::MAX,
            STANDARD_GAS_PRICE,
        );
        assert_charge_succeeds(&mut gas_meter, GasUnit::<2>::from([REMAINING_FUNDS / 2; 2]));
        assert_gas_used_equals(&gas_meter, GasUnit::from([REMAINING_FUNDS / 2; 2]));
        assert_gas_price_equals(&gas_meter, STANDARD_GAS_PRICE);

        assert_charge_fails(&mut gas_meter, GasUnit::<2>::from([1; 2]));
    }

    #[test]
    fn test_charge_without_funds_tracks_usage() {
        let remaining_gas = GasUnit::<2>::from([100; 2]);
        let mut gas_meter = BasicGasMeter::<S>::new_with_gas(remaining_gas, STANDARD_GAS_PRICE);

        assert_charge_succeeds(&mut gas_meter, remaining_gas);
        assert_gas_used_equals(&gas_meter, remaining_gas);
        assert_gas_price_equals(&gas_meter, STANDARD_GAS_PRICE);

        assert_charge_fails(&mut gas_meter, GasUnit::<2>::from([1; 2]));
    }

    #[test]
    fn test_charge_gas_prevents_gas_value_overflow() {
        let remaining_gas = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        let mut gas_meter = BasicGasMeter::<S>::new_with_gas(remaining_gas, MAX_GAS_PRICE);

        let gas = GasUnit::<2>::from([2; 2]);
        let res = gas_meter.charge_gas(gas);

        assert_eq!(
            res,
            Err(GasMeteringError::Overflow(
                "Charge Gas: Unable to charge gas, because the calculation overflows".to_string()
            ))
        );
    }

    #[test]
    fn test_charge_gas_prevents_funds_value_overflow() {
        let remaining_gas = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        let mut gas_meter = BasicGasMeter::<S>::new_with_funds_and_gas(
            Amount::new(u64::MAX as u128),
            remaining_gas,
            MAX_GAS_PRICE,
        );

        let gas = GasUnit::<2>::from([2; 2]);
        let res = gas_meter.charge_gas(gas);

        assert_eq!(
            res,
            Err(GasMeteringError::Overflow(
                "Charge Funds: Unable to charge gas, because the calculation overflows".to_string()
            ))
        );
    }

    #[test]
    fn test_charge_gas_atomic_update_on_failure() {
        let remaining_gas = GasUnit::<2>::from([5, 5]);
        let remaining_funds = Amount::new(1000000);
        let gas_price = GasPrice::<2>::from([Amount::new(10); 2]);

        let mut gas_meter =
            BasicGasMeter::<S>::new_with_funds_and_gas(remaining_funds, remaining_gas, gas_price);

        let gas = GasUnit::<2>::from([10; 2]);
        let res = gas_meter.charge_gas(gas);

        // We have enough funds to charge but not enough gas.
        assert!(res.is_err());
        assert_eq!(gas_meter.remaining_funds, Some(remaining_funds));
        assert_eq!(gas_meter.remaining_gas, remaining_gas);
    }

    #[test]
    fn test_charge_linear_gas_succeeds() {
        let remaining_gas = GasUnit::<2>::from([1000; 2]);
        let mut gas_meter = BasicGasMeter::<S>::new_with_gas(remaining_gas, STANDARD_GAS_PRICE);

        let base_gas = GasUnit::<2>::from([10; 2]);
        let result = gas_meter.charge_linear_gas(base_gas, 5);

        assert!(result.is_ok());
        // Should have charged 10 * 5 = 50 per dimension
        assert_gas_used_equals(&gas_meter, GasUnit::<2>::from([50; 2]));
    }

    #[test]
    fn test_charge_linear_gas_with_zero_parameter() {
        let remaining_gas = GasUnit::<2>::from([100; 2]);
        let mut gas_meter = BasicGasMeter::<S>::new_with_gas(remaining_gas, STANDARD_GAS_PRICE);

        let base_gas = GasUnit::<2>::from([10; 2]);
        let result = gas_meter.charge_linear_gas(base_gas, 0);

        assert!(result.is_ok());
        // Should have charged nothing
        assert_gas_used_equals(&gas_meter, GasUnit::<2>::ZEROED);
    }

    #[test]
    fn test_charge_linear_gas_overflow() {
        let remaining_gas = GasUnit::<2>::from([u64::MAX; 2]);
        let mut gas_meter = BasicGasMeter::<S>::new_with_gas(remaining_gas, STANDARD_GAS_PRICE);

        let base_gas = GasUnit::<2>::from([u64::MAX; 2]);
        let result = gas_meter.charge_linear_gas(base_gas, 2);

        assert!(result.is_err());
        assert!(matches!(result, Err(GasMeteringError::Overflow(_))));
    }

    #[test]
    fn test_charge_linear_gas_insufficient_gas() {
        let remaining_gas = GasUnit::<2>::from([50; 2]);
        let mut gas_meter = BasicGasMeter::<S>::new_with_gas(remaining_gas, STANDARD_GAS_PRICE);

        let base_gas = GasUnit::<2>::from([10; 2]);
        let result = gas_meter.charge_linear_gas(base_gas, 10);

        // Trying to charge 10 * 10 = 100, but only have 50
        assert!(result.is_err());
        assert!(matches!(result, Err(GasMeteringError::OutOfGas { .. })));
    }
}
