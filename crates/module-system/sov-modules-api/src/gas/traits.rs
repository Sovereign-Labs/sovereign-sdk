//! Gas unit definitions and implementations.

use crate::{impl_gas_array, impl_gas_dimensions, impl_gas_unit, impl_serde};
use core::fmt::{self, Debug, Display};

use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_modules_macros::config_value_private;
use sov_universal_wallet::schema::UniversalWallet;
use thiserror::Error;

use crate::{Amount, BasicGasMeter, GasPrice, GasUnit, Spec};

pub(crate) const GAS_DIMENSIONS: usize = config_value_private!(
    "GAS_DIMENSIONS",
    "Couldn't parse `GAS_DIMENSIONS` in TOML file; must be a constant integer (e.g. `GAS_DIMENSIONS = { const = 2 }`)"
);

/// A multi-dimensional gas unit represented as an array of `u64`.
pub trait GasArray:
    'static
    + fmt::Debug
    + Display
    + Clone
    + Send
    + Sync
    + Unpin
    + PartialEq
    + Eq
    + JsonSchema
    + core::hash::Hash
    + Serialize
    + DeserializeOwned
    + BorshSerialize
    + BorshDeserialize
    + UniversalWallet
    + From<[Self::Scalar; GAS_DIMENSIONS]>
    + Into<[Self::Scalar; GAS_DIMENSIONS]>
    + AsRef<[Self::Scalar; GAS_DIMENSIONS]>
    + AsMut<[Self::Scalar; GAS_DIMENSIONS]>
    + TryFrom<Vec<Self::Scalar>, Error: Into<anyhow::Error> + Debug>
{
    /// A zeroed instance of the unit.
    const ZEROED: Self;

    /// The maximum value of the gas unit.
    const MAX: Self;

    /// The scalar type of the gas unit. Typically u64 or u128.
    type Scalar;

    /// Returns the sum of the two gas units or None if the result overflows.
    fn checked_combine(&self, rhs: &Self) -> Option<Self>;

    /// Out-of-place substraction of gas units.
    ///
    /// # Output
    /// Returns [`None`] if the substraction in any gas dimension underflows.
    fn checked_sub(&self, rhs: &Self) -> Option<Self>;

    /// Returns the product of the scalar and the gas units or None if the result overflows.
    fn checked_scalar_product(&self, scalar: Self::Scalar) -> Option<Self>;

    /// Checks if the gas is less than the provided gas in each dimension of the gas array.
    fn dim_is_less_than(&self, rhs: &Self) -> bool;

    /// Checks if the gas is less or equal to the provided gas in each dimension of the gas array.
    fn dim_is_less_or_eq(&self, rhs: &Self) -> bool;

    /// Calculates the minimum gas values between two gas arrays along each dimension.
    fn calculate_min(lhs: &Self, rhs: &Self) -> Self;

    /// In-place division of gas units.
    fn scalar_division(&mut self, scalar: Self::Scalar) -> &mut Self;

    #[cfg(feature = "test-utils")]
    /// In-place addition of gas units with a scalar.
    fn scalar_add(&mut self, scalar: Self::Scalar) -> &mut Self;

    #[cfg(feature = "test-utils")]
    /// In-place substraction of gas units with a scalar.
    fn scalar_sub(&mut self, scalar: Self::Scalar) -> &mut Self;
}

/// A unit of gas
pub trait Gas: GasArray<Scalar = u64> + TryFrom<Vec<u64>> + From<[u64; GAS_DIMENSIONS]> {
    /// The price of the gas, expressed in tokens per unit.
    type Price: GasArray<Scalar = Amount>;

    /// Calculates the value of the given amount of gas at the given price or returns None if the result overflows.
    fn checked_value(&self, price: &Self::Price) -> Option<Amount>;

    /// Calculates the value of the given amount of gas at the given price.
    fn value(&self, price: &Self::Price) -> Amount;

    /// Returns a gas unit which is zero in all dimensions.
    #[must_use]
    fn zero() -> Self {
        Self::ZEROED
    }

    /// Returns the maximum gas unit.
    #[must_use]
    fn max() -> Self {
        Self::MAX
    }

    #[cfg(feature = "gas-constant-estimation")]
    /// Returns an optional name of the gas unit.
    fn name(&self) -> &Option<String>;

    #[cfg(feature = "gas-constant-estimation")]
    /// Names the gas unit.
    fn with_name(self, name: String) -> Self;
}

impl_gas_unit!(GAS_DIMENSIONS);

/// Error type that can be raised by the `GasMeter` trait.
/// Errors can be raised either when the meter runs out of gas or when the refund operation fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GasMeteringError<GU: Gas> {
    #[error("Gas calculation overflow: {0}")]
    /// Unable to calculate gas usage due to overflow.
    Overflow(String),
    /// The slot gas limit has been exhausted.
    #[error("The slot gas limit has been exhausted. Initial slot gas: {initial_slot_gas}, gas to charge {gas_to_charge}, remaining total slot gas {remaining_total_slot_gas}, remaining preferred slot gas {remaining_preferred_slot_gas}, is preffred {is_preferred}")]
    SlotOutOfGas {
        /// The initial slot gas limit.
        initial_slot_gas: GU,
        /// The amount of gas to charge.
        gas_to_charge: GU,
        /// The remaining preffered gas.
        remaining_preferred_slot_gas: GU,
        /// The remaining total slot gas.
        remaining_total_slot_gas: GU,
        /// Gas allocated to transactions from preferred sequencer.
        is_preferred: bool,
    },
    /// Unable to deserialize data due to invalid length.
    #[error("Unable to deserialize data due to invalid length: {0}")]
    InvalidLength(String),
    /// The gas meter has ran out of gas.
    #[error("The gas to charge is greater than the funds available in the meter. Gas to charge {gas_to_charge}, gas price {gas_price}, initial_gas {initial_gas}, remaining gas {remaining_gas}")]
    OutOfGas {
        /// The amount of gas to charge.
        gas_to_charge: GU,
        /// The current gas price.
        gas_price: GU::Price,
        /// The initial gas.
        initial_gas: GU,
        /// The remaining gas.
        remaining_gas: GU,
    },
    /// The gas meter has ran out of funds.
    #[error("The amount to charge is greater than the funds available in the meter. Amount to charge {amount_to_charge}, remaining_funds  {remaining_funds}, price {gas_price}")]
    OutOfFunds {
        /// The amount to charge.
        amount_to_charge: Amount,
        /// Remaining funds.
        remaining_funds: Amount,
        /// The current gas price.
        gas_price: GU::Price,
    },
    /// The refund operation failed for the gas meter.
    #[error("The gas to refund is greater than the gas used. Gas to refund {gas_to_refund}, gas used {gas_used}")]
    ImpossibleToRefundGas {
        /// Amount of gas to refund.
        gas_to_refund: GU,
        /// Amount of gas currently used by the meter.
        gas_used: GU,
    },
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

/// A type-safe trait that should track the gas consumed by a finite ressource over time.
pub trait GasMeter {
    /// The spec used by this gas meter.
    type Spec: Spec;

    /// Charges a fix amount of gas in the gas meter.
    ///
    /// # Errors
    /// Raises an error if the gas to charge is greater than the funds available, or if
    /// calculating the price of the gas overflows
    fn charge_gas(
        &mut self,
        _amount: &<Self::Spec as Spec>::Gas,
    ) -> Result<(), GasMeteringError<<Self::Spec as Spec>::Gas>> {
        Ok(())
    }

    /// Charges an amount of gas equal to `amount *_point parameter`, the pointwise product of `amount` times `parameter`.
    ///
    /// # Errors
    /// Raises an error if the gas to charge is greater than the funds available, or if
    /// calculating the price of the gas overflows
    fn charge_linear_gas(
        &mut self,
        _amount: &<Self::Spec as Spec>::Gas,
        _parameter: u32,
    ) -> Result<(), GasMeteringError<<Self::Spec as Spec>::Gas>> {
        Ok(())
    }

    /// Returns the basic gas state if it's a BasicGasMeter or contains a BasicGasMeter. Used to set the EVM gas limit
    fn try_as_basic_gas_meter(&mut self) -> Option<&mut BasicGasMeter<Self::Spec>> {
        None
    }

    /// Tracks the removal of gas consumption pattern.
    /// This is for use only in benchmarks.
    #[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
    fn remove_gas_pattern(&mut self, _amount: &<Self::Spec as Spec>::Gas, _parameter: u32) {}
}

/// Get gas price
pub trait GetGasPrice {
    /// The spec used by this gas meter.
    type Spec: Spec;

    /// Returns gas price.
    fn gas_price(&self) -> &<<Self::Spec as Spec>::Gas as Gas>::Price;
}

/// Represents a mathematical fraction with a numerator and a denominator.
pub struct Fraction {
    /// Numerator
    pub numerator: u32,
    /// Denominator
    /// SAFETY: should be bigger than the numerator.
    pub denominator: u32,
}

impl Fraction {
    const fn preferred_data_fraction() -> Self {
        // SAFETY: Denominator is bigger than numerator.
        Self {
            numerator: 9,
            denominator: 10,
        }
    }
}

/// The maximum portion of the resource allocated to the preferred sequencer.
/// This can refer to either slot space or slot gas limit.
pub const PREFERRED_DATA_FRACTION: Fraction = Fraction::preferred_data_fraction();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_value_test() {
        let gas = GasUnit::<2>::from([10, 20]);
        let gas_price = GasPrice::<2>::from([Amount::new(3), Amount::new(5)]);

        let value = gas.checked_value(&gas_price).unwrap();
        assert_eq!(value, 130);

        let gas = GasUnit::<2>::from([u64::MAX, 20]);
        let gas_price = GasPrice::<2>::from([Amount::new(3), Amount::new(5)]);

        let value = gas.checked_value(&gas_price);
        assert_eq!(value.unwrap(), (u64::MAX as u128) * 3 + 100);

        let gas = GasUnit::<2>::from([u64::MAX, 20]);
        let gas_price = GasPrice::<2>::from([
            Amount::new(u64::MAX as u128)
                .checked_mul(Amount::new(3))
                .unwrap(),
            Amount::new(5),
        ]);

        let value = gas.checked_value(&gas_price);
        assert!(value.is_none());

        let gas = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        let gas_price = GasPrice::<2>::from([Amount::from(u64::MAX); 2]);
        let value = gas.checked_value(&gas_price);
        assert!(value.is_none());

        let gas = GasUnit::<2>::from([0, 10]);
        let gas_price = GasPrice::<2>::from([Amount::MAX, Amount::new(20)]);

        let value = gas.checked_value(&gas_price).unwrap();
        assert_eq!(value, 200);
    }
}
