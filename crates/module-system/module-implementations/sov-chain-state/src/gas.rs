use std::cmp::max;

use serde::{Deserialize, Serialize};
use sov_modules_api::macros::config_value;
use sov_modules_api::{Amount, Gas, GasArray, GasSpec, Spec};
use thiserror::Error;

use crate::{BlockGasInfo, ChainState};

/// A non-zero `u8` ratio, useful for defining ratios and multiplicative constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NonZeroRatio(u8);

/// An error that is returned if attempting to create a [`NonZeroRatio`] from `0_u8`.
#[derive(Error, Debug)]
#[error("This value cannot be set to 0")]
pub struct NonZeroRatioConversionError;

impl NonZeroRatio {
    /// Creates a new [`NonZeroRatio`] from a [`u8`]. This method should be used to build constants of type [`NonZeroRatio`].
    /// To build a [`NonZeroRatio`] at runtime, use the [`TryFrom<u8>`] trait.
    ///
    /// # Safety
    /// This method panics if the provided value is `0`.
    pub const fn from_u8_unwrap(value: u8) -> Self {
        if value == 0 {
            panic!("This value cannot be set to 0");
        }
        Self(value)
    }

    /// Divides the provided value by the ratio.
    pub fn apply_div(&self, value: u128) -> u128 {
        value
            .checked_div(self.0.into())
            .expect("The ratio cannot be zero")
    }

    /// Divides the provided value by the ratio.
    pub fn apply_div_u64(&self, value: u64) -> u64 {
        value
            .checked_div(self.0.into())
            .expect("The ratio cannot be zero")
    }

    /// Gets the ratio as a `u8`.
    pub fn get(&self) -> u8 {
        self.0
    }
}

impl From<NonZeroRatio> for u64 {
    fn from(ratio: NonZeroRatio) -> u64 {
        ratio.0.into()
    }
}

impl TryFrom<u8> for NonZeroRatio {
    type Error = NonZeroRatioConversionError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(NonZeroRatioConversionError)
        } else {
            Ok(Self(value))
        }
    }
}

impl From<NonZeroRatio> for u8 {
    fn from(value: NonZeroRatio) -> Self {
        value.0
    }
}

/// Defines constants used by the chain state module for gas price computation
impl<S: Spec> ChainState<S> {
    /// Configuration value used to control the variations of the base fee updates.
    ///
    /// This is sourced from the `constants.toml` file and then converted into a
    /// [`NonZeroRatio`].
    pub fn config_base_fee_change_denominator() -> NonZeroRatio {
        NonZeroRatio::from_u8_unwrap(config_value!("BASE_FEE_MAX_CHANGE_DENOMINATOR"))
    }

    /// Configuration value used to control the range of variation of the gas price elasticity.
    ///
    /// This is sourced from the `constants.toml` file and then converted into a
    /// [`NonZeroRatio`].
    pub fn config_elasticity_multiplier() -> NonZeroRatio {
        NonZeroRatio::from_u8_unwrap(config_value!("ELASTICITY_MULTIPLIER"))
    }

    /// Computes the gas target for the provided gas limit.
    /// Basically, divides each dimension of the gas limit by the [`ChainState::config_elasticity_multiplier`].
    pub fn gas_target(gas_limit: &S::Gas) -> S::Gas {
        let mut gas_target = gas_limit.clone();
        gas_target.scalar_division(Self::config_elasticity_multiplier().into());

        gas_target
    }

    /// Computes the initial gas target (genesis block) by calling [`ChainState::gas_target`] on the initial gas limit.
    pub fn initial_gas_target() -> S::Gas {
        Self::gas_target(&<S as GasSpec>::initial_gas_limit())
    }
}

impl<S: Spec> ChainState<S> {
    /// Computes the updated gas price following a block execution for a single dimension.
    /// This reproduces the logic of the EIP-1559 specification to compute the updated `base_fee_per_gas` (`<https://eips.ethereum.org/EIPS/eip-1559>`).
    /// Note that here we drop the `parent` prefix and call the state variables `gas_limit`, `gas_used` and `base_fee_per_gas`.
    pub(crate) fn compute_base_fee_per_gas_unidimensional(
        gas_limit: u64,
        gas_used: u64,
        mut base_fee_per_gas: Amount,
    ) -> Amount {
        // The gas target is equal to `gas_limit // config_elasticity_multiplier(`
        let gas_target = Self::config_elasticity_multiplier().apply_div_u64(gas_limit);
        assert!((Self::config_base_fee_change_denominator().get() as u64).checked_mul(gas_target).is_some(), "Misconfiguration: The product of gas_target * baseconfig_base_fee_change_denominator must not excueed u64::MAX");

        if gas_used == gas_target {
            // We reached the gas target, so we don't need to update the base fee
            base_fee_per_gas
        } else {
            // We need to update the base fee because we didn't reach the gas target.

            // Compute the difference in absolute value between the gas target and the gas used.
            // This value is the delta between the gas target and the gas used. We need to then apply the `base_fee_per_gas` to compute its value in tokens.
            let gas_used_delta = gas_target.abs_diff(gas_used);
            // .checked_mul(base_fee_per_gas.0)
            //.checked_div(gas_target)

            fn compute_delta_limbs(
                gas_used_delta: u64,
                gas_target: u64,
                base_fee_per_gas: u128,
                base_fee_change_denominator: u8,
            ) -> u128 {
                let hi = base_fee_per_gas >> 64;
                let lo = base_fee_per_gas & u64::MAX as u128;

                let base_fee_change_denominator: u128 = base_fee_change_denominator.into();
                // Our divisor is gas target * 8, which is bounded by u64::MAX as long as gas_target * base_fee_change_denominator <= u64::MAX
                let divisor: u128 = (gas_target as u128)
                    .checked_mul(base_fee_change_denominator)
                    // Safety: This can't overflow since the values are bounded by u64::MAX and u8::MAX respectively
                    .unwrap();

                let hi_mul = hi * gas_used_delta as u128;
                let hi_res = hi_mul / divisor;
                // If we would overflow when shifting left, return the max value.
                if hi_res > u64::MAX as u128 {
                    return u128::MAX;
                }
                let hi_rem = hi_mul % divisor;
                let low_mul = lo * gas_used_delta as u128;
                let low_res = low_mul / divisor;

                // This is correct as long as divisor <= u64::MAX
                (hi_res << 64)
                    .saturating_add((hi_rem << 64) / divisor)
                    .saturating_add(low_res)
            }
            let base_fee_per_gas_delta_normalized = compute_delta_limbs(
                gas_used_delta,
                gas_target,
                base_fee_per_gas.0,
                Self::config_base_fee_change_denominator().get(),
            );

            // This division expresses the `base_fee_per_gas` delta as the ration (gas_used_delta_value / gas_target).
            // If the division underflows, the delta is set to zero
            //
            // Note here that this operation gives a value that can be expressed as a `GasPrice<1>` because we do
            // `base_fee_per_gas * (gas_used_delta / gas_target)`.

            // We normalize the result, the same way as in the EIP-1559 specification (`<https://eips.ethereum.org/EIPS/eip-1559>`)

            if gas_used > gas_target {
                // In that case, we take the maximum with `1` to make sure the `base_fee_per_gas` is always increased
                let base_fee_per_gas_delta_normalized =
                    Amount::from(max(base_fee_per_gas_delta_normalized, 1));

                base_fee_per_gas =
                    base_fee_per_gas.saturating_add(base_fee_per_gas_delta_normalized);

                base_fee_per_gas
            } else {
                // Although unlikely, the `base_fee_per_gas` can reach zero. We cannot have a negative value for gas price
                // so we saturate at zero.
                base_fee_per_gas.saturating_sub(Amount::from(base_fee_per_gas_delta_normalized))
            }
        }
    }

    /// Computes the gas price for the a slot given it's parent's gas consumption and the number of *slots* elapsed since
    /// the parent *block* was executed.
    ///
    /// The computation of the base price for the current block is determined
    /// by the value of the `base_fee_per_gas`, `gas_limit` of the parent block as well as constant parameters
    /// such as the [`ChainState::config_elasticity_multiplier`] and the amount of gas used in the parent block.
    /// The computation follows the one described in the EIP-1559 specification, where each dimension
    /// of the multi-dimensional gas price is independently updated following EIP-1559.
    pub fn compute_base_fee_per_gas(
        mut parent_gas_info: BlockGasInfo<S::Gas>,
        slots_since_last_update: u64,
    ) -> <S::Gas as Gas>::Price {
        // We need to compute the base fee per gas for the slots that are not yet visible to us in state, starting from the previous rollup height.
        // We just iteratively compute the base fee per gas for each slot assuming zero gas used and a constant gas limit.
        for _ in 0..slots_since_last_update {
            let next_base_price = Self::compute_base_fee_update_for_slot(&parent_gas_info);
            // TODO(@theochap): the gas limit should be updated dynamically `<https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/271`
            // This TODO is for performance enhancement, not a security concern. Updating the gas limit dynamically would allow
            // the work of the prover to follow high level industry trends of the costs to compute zk-proofs.
            parent_gas_info = BlockGasInfo::new(S::initial_gas_limit(), next_base_price);
        }
        parent_gas_info.base_fee_per_gas().clone()
    }

    fn compute_base_fee_update_for_slot(
        parent_gas_info: &BlockGasInfo<S::Gas>,
    ) -> <S::Gas as Gas>::Price {
        let mut output = parent_gas_info.base_fee_per_gas().clone();
        output
            .as_mut()
            .iter_mut()
            .zip(parent_gas_info.gas_limit().as_ref().iter())
            .zip(parent_gas_info.gas_used().as_ref().iter())
            .for_each(|((base_fee_per_gas, gas_limit), gas_used)| {
                *base_fee_per_gas = Self::compute_base_fee_per_gas_unidimensional(
                    *gas_limit,
                    *gas_used,
                    *base_fee_per_gas,
                );
            });

        output
    }
}
