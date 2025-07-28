use sov_modules_api::Amount;
use sov_test_utils::TestSpec;

use crate::ChainState;

struct BaseFeeUpdateConfig {
    gas_limit: u64,
    gas_used: u64,
    base_fee_per_gas: Amount,
}

/// These tests ensure that the unidimensional base fee per gas update function has the correct behavior
fn assert_correct_gas_update(
    prev_block_gas_info: BaseFeeUpdateConfig,
    expected_base_fee_per_gas: Amount,
    assert_error_message: &str,
) {
    let computed_base_fee_per_gas = ChainState::<TestSpec>::compute_base_fee_per_gas_unidimensional(
        prev_block_gas_info.gas_limit,
        prev_block_gas_info.gas_used,
        prev_block_gas_info.base_fee_per_gas,
    );
    assert_eq!(
        expected_base_fee_per_gas,
        computed_base_fee_per_gas,
        "The result of `compute_base_fee_per_gas_unidimensional` does not match the expected value. 
        Expected: {expected_base_fee_per_gas:?}, actual: {computed_base_fee_per_gas:?}, test message: {assert_error_message}"
    );
}

/// The base fee per gas should remain to zero if the gas used is below the target
#[test]
fn test_zero_base_fee_gas_below_target() {
    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit: 0,
            gas_used: 0,
            base_fee_per_gas: Amount::ZERO,
        },
        Amount::ZERO,
        "When the base fee per gas is zero, it should remain to zero if the gas used is below the target",
    );
}

/// The base fee per gas should increase from zero to 1 if the gas used is above the target
#[test]
fn test_zero_base_fee_gas_above_target() {
    const GAS_LIMIT: u64 = 100;
    let gas_target: u64 = ChainState::<TestSpec>::config_elasticity_multiplier()
        .apply_div(GAS_LIMIT as u128)
        .try_into()
        .unwrap();

    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit: GAS_LIMIT,
            gas_used: gas_target + 1,
            base_fee_per_gas: Amount::ZERO,
        },
       Amount::new(1),
        "When the base fee per gas is zero, it should be equal to one if the gas used is above the target",
    );
}

/// The base fee per gas should not change if the gas used is the same as the target
#[test]
fn test_base_fee_target_reached() {
    const INITIAL_BASE_FEE_PER_GAS: Amount = Amount::new(10000);
    const GAS_LIMIT: u64 = 100;
    let gas_target: u64 = ChainState::<TestSpec>::config_elasticity_multiplier()
        .apply_div(GAS_LIMIT as u128)
        .try_into()
        .unwrap();

    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit: GAS_LIMIT,
            gas_used: gas_target,
            base_fee_per_gas: INITIAL_BASE_FEE_PER_GAS,
        },
        INITIAL_BASE_FEE_PER_GAS,
        "When the gas target is met, the base fee per gas shouldn't change",
    );
}

/// The base fee per gas should increase by `base_fee_per_gas * 1/config_base_fee_change_denominator()` if the gas used is twice as much as the target
#[test]
fn test_base_fee_increases_if_gas_used_is_twice_target() {
    const INITIAL_BASE_FEE_PER_GAS: u128 = 100;

    let expected_base_fee_per_gas: u128 = INITIAL_BASE_FEE_PER_GAS
        + ChainState::<TestSpec>::config_base_fee_change_denominator()
            .apply_div(INITIAL_BASE_FEE_PER_GAS);

    let gas_limit: u64 = 100;
    let gas_target: u64 = ChainState::<TestSpec>::config_elasticity_multiplier()
        .apply_div(gas_limit as u128)
        .try_into()
        .unwrap();

    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit,
            gas_used: 2 * gas_target,
            base_fee_per_gas: INITIAL_BASE_FEE_PER_GAS.into(),
        },
        expected_base_fee_per_gas.into(),
        "The base fee per gas should increase by `base_fee_per_gas * (1/config_base_fee_change_denominator())` if the gas used is twice as much as the target",
    );

    // The new value of the base fee should not depend on the value of the gas used, as long as it is twice as much as the target
    let new_gas_limit: u64 = 100 * gas_limit;
    let new_gas_target: u64 = ChainState::<TestSpec>::config_elasticity_multiplier()
        .apply_div(new_gas_limit as u128)
        .try_into()
        .unwrap();

    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit: new_gas_limit,
            gas_used: 2 * new_gas_target,
            base_fee_per_gas: INITIAL_BASE_FEE_PER_GAS.into(),
        },
        expected_base_fee_per_gas.into(),
        "The base fee per gas should increase by `base_fee_per_gas * (1/config_base_fee_change_denominator())` if the gas used is twice as much as the target. The new base fee should not depend on the value of the gas used, as long as it is twice as much as the target",
    );
}

/// The base fee per gas should increase by `base_fee_per_gas * 1/config_base_fee_change_denominator()` if the gas used is twice as much as the target
#[test]
fn test_base_fee_increases_if_gas_used_is_twice_target_near_overflow() {
    const INITIAL_BASE_FEE_PER_GAS: u128 = u128::MAX / 2;

    let expected_base_fee_per_gas: u128 = INITIAL_BASE_FEE_PER_GAS
        + ChainState::<TestSpec>::config_base_fee_change_denominator()
            .apply_div(INITIAL_BASE_FEE_PER_GAS);

    let gas_limit: u64 = u64::MAX / 8;
    let gas_target: u64 = ChainState::<TestSpec>::config_elasticity_multiplier()
        .apply_div(gas_limit as u128)
        .try_into()
        .unwrap();

    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit,
            gas_used: 2 * gas_target,
            base_fee_per_gas: INITIAL_BASE_FEE_PER_GAS.into(),
        },
        expected_base_fee_per_gas.into(),
        "The base fee per gas should increase by `base_fee_per_gas * (1/config_base_fee_change_denominator())` if the gas used is twice as much as the target",
    );

    // The new value of the base fee should not depend on the value of the gas used, as long as it is twice as much as the target
    let new_gas_limit: u64 = 100;
    let new_gas_target: u64 = ChainState::<TestSpec>::config_elasticity_multiplier()
        .apply_div(new_gas_limit as u128)
        .try_into()
        .unwrap();

    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit: new_gas_limit,
            gas_used: 2 * new_gas_target,
            base_fee_per_gas: INITIAL_BASE_FEE_PER_GAS.into(),
        },
        expected_base_fee_per_gas.into(),
        "The base fee per gas should increase by `base_fee_per_gas * (1/config_base_fee_change_denominator())` if the gas used is twice as much as the target. The new base fee should not depend on the value of the gas used, as long as it is twice as much as the target",
    );
}

/// The base fee per gas should increase by `base_fee_per_gas * 1/config_base_fee_change_denominator()` if the gas used is twice as much as the target
#[test]
fn test_base_fee_increase_saturates_at_max() {
    const INITIAL_BASE_FEE_PER_GAS: u128 = u128::MAX - 1;

    let gas_limit: u64 = u64::MAX / 8;
    let gas_target: u64 = ChainState::<TestSpec>::config_elasticity_multiplier()
        .apply_div(gas_limit as u128)
        .try_into()
        .unwrap();

    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit,
            gas_used: 2 * gas_target,
            base_fee_per_gas: INITIAL_BASE_FEE_PER_GAS.into(),
        },
        Amount::MAX,
        "The base fee per gas should saturate at the maximum value",
    );
}

/// The base fee per gas should increase by `base_fee_per_gas * 1/config_base_fee_change_denominator()` if the gas used is twice as much as the target
#[test]
fn test_base_fee_increase_handles_remainder_correctly() {
    const INITIAL_BASE_FEE_PER_GAS: u128 = (u64::MAX as u128) + 11;

    let expected_base_fee_per_gas: u128 = INITIAL_BASE_FEE_PER_GAS
        + ChainState::<TestSpec>::config_base_fee_change_denominator()
            .apply_div(INITIAL_BASE_FEE_PER_GAS);

    let gas_limit: u64 = u64::MAX / 8;
    let gas_target: u64 = ChainState::<TestSpec>::config_elasticity_multiplier()
        .apply_div(gas_limit as u128)
        .try_into()
        .unwrap();

    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit,
            gas_used: 2 * gas_target,
            base_fee_per_gas: INITIAL_BASE_FEE_PER_GAS.into(),
        },
        expected_base_fee_per_gas.into(),
        "The base fee per gas should increase by `base_fee_per_gas * (1/config_base_fee_change_denominator())` if the gas used is twice as much as the target",
    );
}

/// The base fee per gas should decrease by `base_fee_per_gas * 1/(2*config_base_fee_change_denominator())` if the gas used is half as much as the target
#[test]
fn base_fee_decreases_if_gas_used_is_half_target() {
    const INITIAL_BASE_FEE_PER_GAS: u128 = 10000;

    let expected_base_fee_per_gas: u128 = INITIAL_BASE_FEE_PER_GAS
        - ChainState::<TestSpec>::config_base_fee_change_denominator()
            .apply_div(INITIAL_BASE_FEE_PER_GAS)
            / 2;

    let gas_limit: u64 = 100;
    let gas_target: u64 = ChainState::<TestSpec>::config_elasticity_multiplier()
        .apply_div(gas_limit as u128)
        .try_into()
        .unwrap();

    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit,
            gas_used: gas_target/2,
            base_fee_per_gas: INITIAL_BASE_FEE_PER_GAS.into(),
        },
        expected_base_fee_per_gas.into(),
        "The base fee per gas should decrease by `base_fee_per_gas * 1/(2*config_base_fee_change_denominator)` if the gas used is half the target",
    );

    // The new value of the base fee should not depend on the value of the gas used, as long as it is half of the target
    let new_gas_limit: u64 = 100 * gas_limit;
    let new_gas_target: u64 = ChainState::<TestSpec>::config_elasticity_multiplier()
        .apply_div(new_gas_limit as u128)
        .try_into()
        .unwrap();

    assert_correct_gas_update(
        BaseFeeUpdateConfig {
            gas_limit: new_gas_limit,
            gas_used: new_gas_target/2,
            base_fee_per_gas: INITIAL_BASE_FEE_PER_GAS.into(),
        },
        expected_base_fee_per_gas.into(),
        "The base fee per gas should increase by `base_fee_per_gas * 1/(2*config_base_fee_change_denominator)` if the gas used is half the target. The new base fee should not depend on the value of the gas used",
    );
}
