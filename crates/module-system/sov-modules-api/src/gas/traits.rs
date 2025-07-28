//! Gas unit definitions and implementations.

use core::fmt::{self, Debug, Display};
use std::cmp::min;

use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_modules_macros::config_value_private;
use sov_universal_wallet::schema::{Container, IndexLinking, Item, Link, Schema, UniversalWallet};
use sov_universal_wallet::ty::{Tuple, UnnamedField};
use thiserror::Error;

use crate::{Amount, DaSpec, Spec};

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

/// A multi-dimensional gas unit.
#[derive(Clone, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize, derive_more::Display)]
#[display("GasUnit{:?}", self.value)]
pub struct GasUnit<const N: usize> {
    value: [u64; N],
    #[cfg(feature = "gas-constant-estimation")]
    #[borsh(skip)]
    name: Option<String>,
}

impl<const N: usize> UniversalWallet for GasUnit<N>
where
    Self: 'static,
    [u64; N]: UniversalWallet,
{
    fn scaffold() -> Item<IndexLinking> {
        Item::Container(Container::Tuple(Tuple {
            template: None,
            peekable: false,
            fields: vec![UnnamedField {
                value: Link::Placeholder,
                silent: false,
                doc: String::new(),
            }],
        }))
    }

    fn get_child_links(schema: &mut Schema) -> Vec<Link> {
        vec![<[u64; N]>::make_linkable(schema)]
    }
}

impl<const N: usize> Debug for GasUnit<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

/// A gas price for multi-dimensional gas.
#[derive(
    Clone,
    PartialEq,
    Eq,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    sov_rollup_interface::sov_universal_wallet::UniversalWallet,
    derive_more::Display,
)]
#[sov_wallet()]
#[display("GasPrice{:?}", self.value)]
pub struct GasPrice<const N: usize> {
    value: [Amount; N],
}

impl<const N: usize> Debug for GasPrice<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

// Implement basic traits for wrappers around [$u; $n] (example: GasPrice is [u128; 2])
macro_rules! impl_gas_dimensions {
    ($t: ty, $t_name: literal, $n: expr, $u: ty) => {
        impl schemars::JsonSchema for $t {
            fn schema_name() -> String {
                $t_name.to_owned() + "(" + &format!("{}", $n) + ")"
            }

            fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
                serde_json::from_value(serde_json::json!({
                    "type": "array",
                    "minItems": $n,
                    "maxItems": $n,
                    "items": {
                        "type": "number"
                    },
                    // This description assumes that `serializer` uses a human-readable format.
                    "description": $t_name.to_owned() + " is an array of size " + &format!("{}", $n),
                }))
                .unwrap()
            }
        }

        impl From<[$u; $n]> for $t {
            fn from(array: [$u; $n]) -> Self {
                Self::from_primitive(array)
            }
        }

        impl From<$t> for [$u; $n] {
            fn from(gas: $t) -> [$u; $n] {
                gas.value
            }
        }

        impl TryFrom<Vec<$u>> for $t {
            type Error = anyhow::Error;

            fn try_from(value: Vec<$u>) -> Result<Self, Self::Error> {
                if value.len() != $n {
                    anyhow::bail!("Impossible to convert to a gas unit. The array must have {} elements, but it has {}", $n, value.len());
                }

                let mut output = [<$u>::from(0u64); $n];
                output.copy_from_slice(&value);

                Ok(Self::from(output))
            }
        }

        impl AsRef<[$u; $n]> for $t {
            fn as_ref(&self) -> &[$u; $n] {
                &self.value
            }
        }

        impl AsMut<[$u; $n]> for $t {
            fn as_mut(&mut self) -> &mut [$u; $n] {
                &mut self.value
            }
        }
    };
}

// Implments the `GasArray` trait for the wrapper around [$u; $n] (example: GasUnit is [u64; 2])
macro_rules! impl_gas_array {
    ($t: ty, $n:expr, $u:ty) => {
        impl GasArray for $t {
            type Scalar = $u;
            // u64::ZERO would be better, but we have const and don't want thirdr party crate.
            const ZEROED: Self = Self::from_primitive([<$u>::MIN; $n]);

            const MAX: Self = Self::from_primitive([<$u>::MAX; $n]);

            fn checked_sub(&self, rhs: &Self) -> Option<Self> {
                let mut output = [<$u>::from(0u64); $n];

                for (i, (l, r)) in self.value.iter().zip(rhs.value.as_slice()).enumerate() {
                    if let Some(res) = l.checked_sub(*r) {
                        output[i] = res;
                    } else {
                        return None;
                    }
                }

                Some(Self::from(output))
            }

            fn checked_scalar_product(&self, scalar: $u) -> Option<Self> {
                let mut output = [<$u>::from(0u64); $n];

                for (i, v) in self.value.iter().enumerate() {
                    if let Some(res) = v.checked_mul(scalar) {
                        output[i] = res;
                    } else {
                        return None;
                    }
                }

                Some(Self::from(output))
            }

            fn dim_is_less_than(&self, rhs: &Self) -> bool {
                for (l, r) in self.value.iter().zip(rhs.value.as_slice()) {
                    if l >= r {
                        return false;
                    }
                }
                true
            }

            fn dim_is_less_or_eq(&self, rhs: &Self) -> bool {
                for (l, r) in self.value.iter().zip(rhs.value.as_slice()) {
                    if l > r {
                        return false;
                    }
                }
                true
            }

            fn calculate_min(lhs: &Self, rhs: &Self) -> Self {
                let mut output = [<$u>::from(0u64); $n];

                for (i, (l, r)) in lhs.value.iter().zip(rhs.value.iter()).enumerate() {
                    output[i] = min(*l, *r);
                }
                Self::from_primitive(output)
            }

            fn scalar_division(&mut self, scalar: $u) -> &mut Self {
                self.value
                    .iter_mut()
                    .for_each(|s| *s = s.checked_div(scalar).unwrap_or(<$u>::from(0u64)));
                self
            }

            #[cfg(feature = "test-utils")]
            fn scalar_add(&mut self, scalar: $u) -> &mut Self {
                self.value
                    .iter_mut()
                    .for_each(|s| *s = s.saturating_add(scalar));
                self
            }

            #[cfg(feature = "test-utils")]
            fn scalar_sub(&mut self, scalar: $u) -> &mut Self {
                self.value
                    .iter_mut()
                    .for_each(|s| *s = s.saturating_sub(scalar));
                self
            }

            fn checked_combine(&self, rhs: &Self) -> Option<Self> {
                let mut output = [<$u>::from(0u64); $n];

                for (i, (l, r)) in self.value.iter().zip(rhs.value.iter()).enumerate() {
                    if let Some(res) = l.checked_add(*r) {
                        output[i] = res;
                    } else {
                        return None;
                    }
                }
                Some(Self::from_primitive(output))
            }
        }
    };
}

macro_rules! impl_gas_unit {
    ($n:expr) => {
        impl Gas for GasUnit<$n> {
            type Price = GasPrice<$n>;

            #[cfg(feature = "gas-constant-estimation")]
            fn name(&self) -> &Option<String> {
                &self.name
            }

            /// Adds a name tag to the gas constant.
            #[cfg(feature = "gas-constant-estimation")]
            fn with_name(self, name: String) -> Self {
                Self {
                    name: Some(name),
                    ..self
                }
            }

            fn checked_value(&self, price: &Self::Price) -> Option<Amount> {
                let mut value: Amount = Amount::ZERO;
                for (g, p) in self.value.iter().zip(price.as_ref().iter().copied()) {
                    let v = Amount::new(*g as u128).checked_mul(p)?;
                    value = value.checked_add(v)?;
                }

                Some(value)
            }

            fn value(&self, price: &Self::Price) -> Amount {
                self.value
                    .iter()
                    .zip(price.as_ref().iter().copied())
                    .map(|(a, b)| Amount::new(*a as u128).saturating_mul(b))
                    .fold(Amount::new(0), |a, b| a.saturating_add(b))
            }
        }

        impl GasUnit<$n> {
            /// Creates a new [`GasUnit`] from an array of [`u64`].
            const fn from_primitive(array: [u64; $n]) -> Self {
                Self {
                    value: array,
                    #[cfg(feature = "gas-constant-estimation")]
                    name: None,
                }
            }
        }

        impl GasPrice<$n> {
            /// Creates a new [`GasPrice`] from an array of Amount.
            #[must_use]
            pub const fn from_primitive(array: [Amount; $n]) -> Self {
                let mut value: [Amount; $n] = [Amount::ZERO; $n];

                let mut i = 0;
                while i < $n {
                    value[i] = array[i];
                    i += 1;
                }

                Self { value }
            }
        }

        impl ::serde::Serialize for GasUnit<$n> {
            fn serialize<__S>(&self, serializer: __S) -> Result<__S::Ok, __S::Error>
            where
                __S: serde::Serializer,
            {
                <[u64; $n] as serde::Serialize>::serialize(&self.value, serializer)
            }
        }

        impl<'de> serde::Deserialize<'de> for GasUnit<$n> {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let array = <[u64; $n] as serde::Deserialize>::deserialize(deserializer)?;
                Ok(Self::from(array))
            }
        }

        impl ::serde::Serialize for GasPrice<$n> {
            fn serialize<__S>(&self, serializer: __S) -> Result<__S::Ok, __S::Error>
            where
                __S: serde::Serializer,
            {
                <[Amount; $n] as serde::Serialize>::serialize(&self.value, serializer)
            }
        }

        impl<'de> serde::Deserialize<'de> for GasPrice<$n> {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let array = <[Amount; $n] as serde::Deserialize>::deserialize(deserializer)?;
                Ok(Self::from(array))
            }
        }

        impl_gas_array!(GasUnit<$n>, $n, u64);
        impl_gas_array!(GasPrice<$n>, $n, Amount);
        impl_gas_dimensions!(GasUnit<$n>, "GasUnit", $n, u64);
        impl_gas_dimensions!(GasPrice<$n>, "GasPrice", $n, Amount);
    };
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

#[macro_export]
/// Defines a constant gas value.
macro_rules! new_constant {
    ($name: literal, $gas: ty) => {{
        #[cfg(feature = "gas-constant-estimation")]
        {
            <$gas>::from(config_value_private!($name)).with_name($name.to_string())
        }
        #[cfg(not(feature = "gas-constant-estimation"))]
        {
            <$gas>::from(config_value_private!($name))
        }
    }};
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

/// A gas meter that tracks the gas used for a slot.
pub struct SlotGasMeter<S: Spec> {
    preferred_sequencer: Option<<S::Da as DaSpec>::Address>,
    initial_slot_gas: S::Gas,
    // Assumption: The preferred batches/proofs are executed before standard batches/proofs.
    remaining_preferred_slot_gas: S::Gas,
    remaining_total_slot_gas: S::Gas,
}

impl<S: Spec> SlotGasMeter<S> {
    /// Creates a new `SlotGasMeter`
    ///
    /// # Panics
    /// May panic with an overflow if the PREFFERERD_DATA_FRACTION is defined to be greater than one,
    /// which is a logic error. Will not panic under normal conditions.
    pub fn new(
        remaining_slot_gas: S::Gas,
        preferred_sequencer: Option<<S::Da as DaSpec>::Address>,
    ) -> Self {
        let remaining_preferred_slot_gas = remaining_slot_gas
            .clone()
            .scalar_division(PREFERRED_DATA_FRACTION.denominator.into())
            .checked_scalar_product(PREFERRED_DATA_FRACTION.numerator.into())
            // This cannot overflow because the PREFERRED_DATA_FRACTION must be less than 1.
            .unwrap();

        Self {
            preferred_sequencer,
            remaining_preferred_slot_gas,
            initial_slot_gas: remaining_slot_gas.clone(),
            remaining_total_slot_gas: remaining_slot_gas,
        }
    }

    /// Get the remaining slot gas.
    pub fn remaining_slot_gas(&self, sequencer: &<S::Da as DaSpec>::Address) -> &S::Gas {
        if Some(sequencer) == self.preferred_sequencer.as_ref() {
            &self.remaining_preferred_slot_gas
        } else {
            &self.remaining_total_slot_gas
        }
    }

    /// Get the remaining gas allowed for the preferred sequencer.
    pub fn remaining_preferred_slot_gas(&self) -> &S::Gas {
        &self.remaining_preferred_slot_gas
    }

    /// Charge gas.
    ///
    /// # Errors
    /// Raises an error if the gas to charge is greater than the funds available
    pub fn charge_gas(
        &mut self,
        gas: &S::Gas,
        sequencer: &<S::Da as DaSpec>::Address,
    ) -> Result<(), GasMeteringError<S::Gas>> {
        // Preferred transactions reduce both the "preferred" gas and the "total" gas,
        // while standard transactions only reduce the `total` gas.
        // The `initial total` gas is always greater than the "initial preferred" gas, and preferred
        // transactions are executed before standard transactions.
        // This mechanism ensures that the preferred sequencer cannot fully censor
        // other sequencers (or emergency registrations) by exhausting all the available gas.
        if Some(sequencer) == self.preferred_sequencer.as_ref() {
            self.remaining_preferred_slot_gas = self
                .remaining_preferred_slot_gas
                .checked_sub(gas)
                .ok_or(self.gas_error(gas.clone(), true))?;
        }

        self.remaining_total_slot_gas = self
            .remaining_total_slot_gas
            .checked_sub(gas)
            .ok_or(self.gas_error(gas.clone(), false))?;

        Ok(())
    }

    /// Total gas used in the slot.
    pub fn total_gas_used(&self) -> S::Gas {
        self.initial_slot_gas
            .checked_sub(&self.remaining_total_slot_gas)
            .expect("The remaining_slot_gas can't be greater than the initial_slot_gas")
    }

    fn gas_error(&self, gas_to_charge: S::Gas, is_preferred: bool) -> GasMeteringError<S::Gas> {
        GasMeteringError::SlotOutOfGas {
            initial_slot_gas: self.initial_slot_gas.clone(),
            gas_to_charge,
            remaining_preferred_slot_gas: self.remaining_preferred_slot_gas.clone(),
            remaining_total_slot_gas: self.remaining_total_slot_gas.clone(),
            is_preferred,
        }
    }
}

/// A struct that keeps track of the gas used.
/// The gas meter continues running until it either depletes its funds or runs out of gas, depending on its configuration.
/// It also ensures that the gas used will not overflow when multiplied by the gas price.
#[derive(Clone, Debug)]
pub struct BasicGasMeter<S: Spec> {
    initial_gas: S::Gas,
    remaining_gas: S::Gas,
    remaining_funds: Option<Amount>,
    gas_price: <S::Gas as Gas>::Price,
}

impl<S: Spec> BasicGasMeter<S> {
    /// Get gas info from the `BasicGasMeter`
    pub fn gas_info(&self) -> GasInfo<S::Gas> {
        let gas_used = self
            .initial_gas
            .checked_sub(&self.remaining_gas)
            .expect("The remaining gas can't be greater than the initial gas");

        let gas_value = gas_used
            .checked_value(&self.gas_price)
            // SAFETY: This is impossible becouse we check for oveflows in `BasicGasMeter::charge_gas_inner`.
            .expect("BasicGasMeter error. The gas value should be possible to compute");

        GasInfo {
            gas_value,
            gas_used,
            gas_price: self.gas_price.clone(),
        }
    }

    /// Creates a new `BasicGasMeter`.
    pub fn new_with_funds_and_gas(
        remaining_funds: Amount,
        remaining_gas: S::Gas,
        gas_price: <S::Gas as Gas>::Price,
    ) -> Self {
        Self {
            initial_gas: remaining_gas.clone(),
            remaining_gas,
            remaining_funds: Some(remaining_funds),
            gas_price,
        }
    }

    /// Creates a new `BasicGasMeter`
    pub fn new_with_gas(remaining_gas: S::Gas, gas_price: <S::Gas as Gas>::Price) -> Self {
        Self {
            initial_gas: remaining_gas.clone(),
            remaining_gas,
            remaining_funds: None,
            gas_price,
        }
    }

    fn compute_remaining_funds(
        &self,
        remaining_funds: Amount,
        amount: &S::Gas,
    ) -> Result<Amount, GasMeteringError<S::Gas>> {
        let amount_value = amount.checked_value(&self.gas_price).ok_or_else(|| {
            GasMeteringError::Overflow(
                "Charge Funds: Unable to charge gas, because the calculation overflows".to_string(),
            )
        })?;

        remaining_funds.checked_sub(amount_value).ok_or_else(|| {
            tracing::warn!(%remaining_funds, amount_to_charge = %amount_value, "Out of gas during `compute_remaining_funds`");
            GasMeteringError::OutOfFunds {
                amount_to_charge: amount_value,
                remaining_funds,
                gas_price: self.gas_price.clone(),
            }
        })
    }

    fn compute_remaining_gas(
        &self,
        remaining_gas: &S::Gas,
        amount: &S::Gas,
    ) -> Result<S::Gas, GasMeteringError<S::Gas>> {
        remaining_gas.checked_sub(amount).ok_or_else(|| {
            tracing::warn!(?remaining_gas, amount_to_charge = ?amount, "Out of gas during `compute_remaining_gas`");
            GasMeteringError::OutOfGas {
                gas_to_charge: amount.clone(),
                gas_price: self.gas_price.clone(),
                initial_gas: self.initial_gas.clone(),
                remaining_gas: self.remaining_gas.clone(),
            }
        })
    }

    fn charge_gas_inner(&mut self, amount: &S::Gas) -> Result<(), GasMeteringError<S::Gas>> {
        let mut new_remaining_funds = None;

        if let Some(remaining_funds) = self.remaining_funds {
            new_remaining_funds = Some(self.compute_remaining_funds(remaining_funds, amount)?);
        }

        let new_remaining_gas = self.compute_remaining_gas(&self.remaining_gas, amount)?;
        // Here we check that the current gas_used won't overflow when multiplied by the price.
        // This ensures that after execution, it is always safe to convert the total gas used to a token value.
        {
            let gas_used = self
                .initial_gas
                .checked_sub(&new_remaining_gas)
                .expect("The remaining gas can't be greater than the initial gas");

            gas_used.checked_value(&self.gas_price).ok_or_else(|| {
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
    fn charge_gas(&mut self, amount: &S::Gas) -> Result<(), GasMeteringError<S::Gas>> {
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
                        var.insert(name.clone(), 1);
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
        amount: &S::Gas,
        parameter: u32,
    ) -> Result<(), GasMeteringError<<S as Spec>::Gas>> {
        let total_amount =
            amount
                .checked_scalar_product(parameter as u64)
                .ok_or(GasMeteringError::Overflow(format!(
                    "Unable to charge gas. The product of {amount} to {parameter} is overflowing"
                )))?;
        tracing::trace!(%total_amount, parameter, gas_before = %self.remaining_gas, funds_before = ?self.remaining_funds, "Charging linear gas");
        self.charge_gas_inner(&total_amount)?;

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
                            var.insert(name.clone(), param_i64);
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
                            var.insert(name.clone(), -param_i64);
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
    fn gas_price(&self) -> &<<Self::Spec as Spec>::Gas as Gas>::Price {
        &self.gas_price
    }
}

#[cfg(test)]
mod tests {
    use sov_mock_da::{MockAddress, MockDaSpec};
    use sov_mock_zkvm::MockZkvm;

    use super::*;
    use crate::default_spec::DefaultSpec;
    use crate::execution_mode::Native;

    type S = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    #[test]
    fn is_less_than_test() {
        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(gas_1.dim_is_less_than(&gas_2));
        assert!(gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([20, 30]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([10, 40]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(!gas_1.dim_is_less_than(&gas_2));
        assert!(!gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([40, 40]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(!gas_1.dim_is_less_than(&gas_2));
        assert!(!gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([40, 40]);
        let gas_2 = GasUnit::<2>::from([20, 50]);
        assert!(!gas_1.dim_is_less_than(&gas_2));
        assert!(!gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([10, 30]);
        assert!(!gas_1.dim_is_less_than(&gas_2));

        let gas_1 = GasUnit::<2>::from([10, 30]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(!gas_1.dim_is_less_than(&gas_2));

        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([10, 30]);
        assert!(gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([10, 30]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(gas_1.dim_is_less_or_eq(&gas_2));
    }

    #[test]
    fn calculate_min_test() {
        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([20, 30]);

        assert_eq!(
            GasUnit::<2>::from([10, 20]),
            GasUnit::calculate_min(&gas_1, &gas_2)
        );

        let gas_1 = GasUnit::<2>::from([20, 30]);
        let gas_2 = GasUnit::<2>::from([10, 20]);

        assert_eq!(
            GasUnit::<2>::from([10, 20]),
            GasUnit::calculate_min(&gas_1, &gas_2)
        );

        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([10, 5]);

        assert_eq!(
            GasUnit::<2>::from([10, 5]),
            GasUnit::calculate_min(&gas_1, &gas_2)
        );

        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([5, 30]);

        assert_eq!(
            GasUnit::<2>::from([5, 20]),
            GasUnit::calculate_min(&gas_1, &gas_2)
        );

        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([10, 20]);

        assert_eq!(
            GasUnit::<2>::from([10, 20]),
            GasUnit::calculate_min(&gas_1, &gas_2)
        );
    }

    #[test]
    fn checked_scalar_product_test() {
        let gas = GasUnit::<2>::from([10, 20]);
        assert_eq!(
            gas.checked_scalar_product(10).unwrap(),
            GasUnit::<2>::from([100, 200]),
        );

        let gas = GasUnit::<2>::from([u64::MAX, 20]);
        assert!(gas.checked_scalar_product(10).is_none());

        let gas = GasUnit::<2>::from([10, u64::MAX]);
        assert!(gas.checked_scalar_product(10).is_none());

        let gas = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        assert!(gas.checked_scalar_product(10).is_none());

        let gas = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        assert_eq!(
            gas.checked_scalar_product(0).unwrap(),
            GasUnit::<2>::from([0, 0]),
        );
    }

    #[test]
    fn checked_combine_test() {
        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([10, 20]);

        assert_eq!(
            gas_1.checked_combine(&gas_2).unwrap(),
            GasUnit::<2>::from([20, 40]),
            "The gas unit should be combined correctly"
        );

        let gas_1 = GasUnit::<2>::from([u64::MAX, 20]);
        let gas_2 = GasUnit::<2>::from([10, 20]);

        assert!(gas_1.checked_combine(&gas_2).is_none());

        let gas_1 = GasUnit::<2>::from([20, 20]);
        let gas_2 = GasUnit::<2>::from([10, u64::MAX]);

        assert!(gas_1.checked_combine(&gas_2).is_none());

        let gas_1 = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        let gas_2 = GasUnit::<2>::from([10, 20]);

        assert!(gas_1.checked_combine(&gas_2).is_none());

        let gas_1 = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        let gas_2 = GasUnit::<2>::from([u64::MAX, u64::MAX]);

        assert!(gas_1.checked_combine(&gas_2).is_none());
    }

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

    #[test]
    fn charge_gas_should_fail_if_not_enough_funds() {
        let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

        {
            let mut gas_meter =
                BasicGasMeter::<S>::new_with_gas(GasUnit::<2>::ZEROED, gas_price.clone());
            assert!(
                gas_meter.charge_gas(&GasUnit::<2>::from([100; 2])).is_err(),
                "The gas meter should not be able to charge gas if there is not enough funds"
            );
        }

        {
            let gas = GasUnit::<2>::from([0, 0]);
            let mut gas_meter = BasicGasMeter::<S>::new_with_gas(gas, gas_price.clone());

            assert!(
                gas_meter.charge_gas(&GasUnit::<2>::from([100; 2])).is_err(),
                "The gas meter should not be able to charge gas if there is not enough gas reserved"
            );

            let gas = GasUnit::<2>::from([1000, 99]);
            let mut gas_meter = BasicGasMeter::<S>::new_with_gas(gas, gas_price.clone());

            assert!(
                gas_meter.charge_gas(&GasUnit::<2>::from([100; 2])).is_err(),
                "The gas meter should not be able to charge gas if there is not enough gas reserved"
            );
        }
    }

    #[test]
    fn try_charge_gas() {
        {
            const REMAINING_FUNDS: u64 = 100;
            let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

            let mut gas_meter = BasicGasMeter::<S>::new_with_funds_and_gas(
                Amount::from(REMAINING_FUNDS),
                GasUnit::<2>::MAX,
                gas_price.clone(),
            );
            assert!(
                gas_meter
                    .charge_gas(&GasUnit::<2>::from([REMAINING_FUNDS / 2; 2]))
                    .is_ok(),
                "It should be possible to charge gas"
            );
            assert_eq!(
                gas_meter.gas_info().gas_used,
                GasUnit::from([REMAINING_FUNDS / 2; 2]),
                "The gas used should be the same as the gas charged"
            );
            assert_eq!(gas_meter.gas_info().gas_price, gas_price);

            assert!(
            gas_meter.charge_gas(&GasUnit::<2>::from([1; 2])).is_err(),
            "There should be no more gas left in the meter, hence charging more gas should fail"
        );
        }

        {
            let remaining_gas = GasUnit::<2>::from([100; 2]);
            let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

            let mut gas_meter =
                BasicGasMeter::<S>::new_with_gas(remaining_gas.clone(), gas_price.clone());

            assert!(
                gas_meter.charge_gas(&remaining_gas.clone()).is_ok(),
                "It should be possible to charge gas"
            );
            assert_eq!(
                gas_meter.gas_info().gas_used,
                remaining_gas,
                "The gas used should be the same as the gas charged"
            );
            assert_eq!(gas_meter.gas_info().gas_price, gas_price);

            assert!(
                gas_meter.charge_gas(&GasUnit::<2>::from([1; 2])).is_err(),
                "There should be no more gas left in the meter, hence charging more gas should fail"
            );
        }
    }

    #[test]
    fn gas_meter_charge_gas_overflow_test() {
        let remaining_gas = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        let gas_price = GasPrice::<2>::from([Amount::MAX; 2]);

        let mut gas_meter =
            BasicGasMeter::<S>::new_with_gas(remaining_gas.clone(), gas_price.clone());

        let gas = GasUnit::<2>::from([2; 2]);
        let res = gas_meter.charge_gas(&gas.clone());

        assert_eq!(
            res,
            Err(GasMeteringError::Overflow(
                "Charge Gas: Unable to charge gas, because the calculation overflows".to_string()
            ))
        );

        let mut gas_meter = BasicGasMeter::<S>::new_with_funds_and_gas(
            Amount::new(u64::MAX as u128),
            remaining_gas.clone(),
            gas_price.clone(),
        );

        let res = gas_meter.charge_gas(&gas);

        assert_eq!(
            res,
            Err(GasMeteringError::Overflow(
                "Charge Funds: Unable to charge gas, because the calculation overflows".to_string()
            ))
        );
    }

    #[test]
    fn gas_meter_charge_atomic_update() {
        let remaining_gas = GasUnit::<2>::from([5, 5]);
        let remaining_funds = Amount::new(1000000);
        let gas_price = GasPrice::<2>::from([Amount::new(10); 2]);

        let mut gas_meter = BasicGasMeter::<S>::new_with_funds_and_gas(
            remaining_funds,
            remaining_gas.clone(),
            gas_price.clone(),
        );

        let gas = GasUnit::<2>::from([10; 2]);
        let res = gas_meter.charge_gas(&gas.clone());

        // We have enough funds to charge but not enough gas.
        assert!(res.is_err());
        assert_eq!(gas_meter.remaining_funds, Some(remaining_funds));
        assert_eq!(gas_meter.remaining_gas, remaining_gas);
    }

    #[test]
    fn slot_gas_meter_test() {
        let mut slot_gas_meter = SlotGasMeter::<S>::new(GasUnit::<2>::from([100, 200]), None);
        let sequencer = MockAddress::new([10; 32]);

        let gas = GasUnit::<2>::from([10, 20]);
        slot_gas_meter.charge_gas(&gas, &sequencer).unwrap();

        assert_eq!(
            slot_gas_meter.remaining_slot_gas(&sequencer),
            &GasUnit::<2>::from([90, 180])
        );

        let preferred_sequencer = MockAddress::new([10; 32]);
        let mut slot_gas_meter =
            SlotGasMeter::<S>::new(GasUnit::<2>::from([100, 200]), Some(preferred_sequencer));

        let sequencer = MockAddress::new([33; 32]);

        let gas = GasUnit::<2>::from([10, 20]);
        slot_gas_meter
            .charge_gas(&gas, &preferred_sequencer)
            .unwrap();

        let expected_preferred_gas = &GasUnit::<2>::from([80, 160]);
        assert_eq!(
            slot_gas_meter.remaining_slot_gas(&preferred_sequencer),
            expected_preferred_gas
        );

        let gas = GasUnit::<2>::from([20, 30]);
        slot_gas_meter.charge_gas(&gas, &sequencer).unwrap();

        assert_eq!(
            slot_gas_meter.remaining_slot_gas(&sequencer),
            &GasUnit::<2>::from([70, 150])
        );

        assert_eq!(
            &slot_gas_meter.remaining_preferred_slot_gas,
            expected_preferred_gas
        );
    }

    #[test]
    fn test_gas_price_serde_json() {
        let gas_price = GasPrice::<2>::from([Amount::new(10); 2]);
        let serialized = serde_json::to_string(&gas_price).unwrap();
        assert_eq!(serialized, r#"["10","10"]"#);

        let deserialized: GasPrice<2> = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, gas_price);
    }

    #[test]
    fn test_gas_price_serde_bincode() {
        let gas_price = GasPrice::<2>::from([Amount::new(10); 2]);
        let serialized = bincode::serialize(&gas_price).unwrap();
        let deserialized: GasPrice<2> = bincode::deserialize(&serialized).unwrap();
        assert_eq!(deserialized, gas_price);
    }
}
