use crate::Receipt;
use anyhow::{bail, ensure, Context};
use sov_modules_api::macros::config_value;
use sov_modules_api::{Gas, GasInfo, Spec};

/// The gas multiplier applied to EVM `gas_limit` based on whether the fee check is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GasMultiplier {
    /// Fee check is active — `gas_limit` maps 1:1 to sovereign gas.
    FeeCheckActive,
    /// Fee check is inactive — sovereign gas budget = `gas_limit * 100`.
    FeeCheckInactive,
}

impl GasMultiplier {
    pub(crate) fn as_u64(self) -> u64 {
        match self {
            Self::FeeCheckActive => 1,
            Self::FeeCheckInactive => 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProjectedReceiptGas {
    pub(crate) gas_used: u64,
    pub(crate) cumulative_gas_used: u64,
}

pub(crate) fn project_receipt_gas_from_actual_fee<S>(
    receipt: &Receipt,
    gas_info: &GasInfo<S::Gas>,
) -> anyhow::Result<Option<ProjectedReceiptGas>>
where
    S: Spec,
{
    let tx_fee_paid = gas_info.gas_value;
    if !is_actual_fee_projection_height_active(receipt.block_number) {
        return Ok(None);
    }

    if tx_fee_paid == sov_bank::Amount::ZERO {
        return Ok(None);
    }

    let current_gas_used = receipt.gas_used;
    // Project charged fee into an EVM receipt gas value:
    // projected = ceil(actual_fee / gas_price[0]), where actual_fee = gas_info.gas_value.
    //
    // Invariant: RPC/reporting uses the block header base fee as effective gas price and it
    // must match this same primary gas-price dimension (`gas_price[0]`). Any divergence is a
    // correctness bug and must fail fast upstream.
    //
    // This intentionally biases upward for non-uniform gas prices so receipt-implied fee
    // is never below the charged fee.
    let projected_gas_used = derive_receipt_gas_used_from_actual_fee(gas_info)?;

    if projected_gas_used == current_gas_used {
        return Ok(None);
    }

    let previous_cumulative = receipt
        .receipt
        .cumulative_gas_used
        .checked_sub(current_gas_used)
        .context("EVM: receipt cumulative gas underflow while projecting from actual fee")?;
    let projected_cumulative = previous_cumulative
        .checked_add(projected_gas_used)
        .context("EVM: receipt cumulative gas overflow while projecting from actual fee")?;

    Ok(Some(ProjectedReceiptGas {
        gas_used: projected_gas_used,
        cumulative_gas_used: projected_cumulative,
    }))
}

pub(crate) fn derive_receipt_gas_used_from_actual_fee<GU: Gas>(
    gas_info: &GasInfo<GU>,
) -> anyhow::Result<u64> {
    let Some(primary_gas_price) = gas_info.gas_price.as_ref().first() else {
        bail!("EVM: gas price vector must have at least one dimension");
    };

    ensure!(
        primary_gas_price.0 > 0,
        "EVM: cannot reconcile receipt from actual fee with zero primary gas price"
    );

    let actual_fee = gas_info.gas_value.0;

    if actual_fee == 0 {
        return Ok(0);
    }

    // UNWRAP: `primary_gas_price.0 > 0` is enforced above.
    let quotient = actual_fee.checked_div(primary_gas_price.0).unwrap();
    // UNWRAP: `primary_gas_price.0 > 0` is enforced above.
    let remainder = actual_fee.checked_rem(primary_gas_price.0).unwrap();
    let projected_gas_used_u128 = if remainder > 0 {
        // Ceiling division: remainder is in fee units, so one extra gas unit is sufficient.
        quotient
            .checked_add(1)
            .context("EVM: projected receipt gas used overflow while rounding up")?
    } else {
        quotient
    };
    let projected_gas_used = u64::try_from(projected_gas_used_u128)
        .context("EVM: projected receipt gas used does not fit in u64")?;

    ensure!(
        projected_gas_used > 0,
        "EVM: projected receipt gas used cannot be zero for non-zero fee"
    );

    Ok(projected_gas_used)
}

/// Returns true once actual-fee projection is enabled for `block_number`.
/// Keep non-height guards in callers; only the activation-height boundary is shared here.
pub(crate) fn is_actual_fee_projection_height_active(block_number: u64) -> bool {
    let apply_actual_fee_after_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
    block_number > apply_actual_fee_after_height
}

/// Returns true once the EIP-1559 max-fee-per-gas check is enabled for `block_number`.
/// Keep non-height guards (e.g. the admin kill-switch) in callers; only the
/// activation-height boundary is shared here.
pub(crate) fn is_max_fee_check_height_active(block_number: u64) -> bool {
    let threshold: u64 = config_value!("EVM_MAX_FEE_CHECK_HEIGHT");
    block_number > threshold
}

#[cfg(test)]
mod tests {
    use super::*;
    use sov_modules_api::GasUnit;

    fn gas_info(gas_used: [u64; 2], gas_price: [sov_bank::Amount; 2]) -> GasInfo<GasUnit<2>> {
        let gas_used = GasUnit::<2>::from(gas_used);
        let gas_price = gas_price.into();
        let gas_value = gas_used
            .checked_value(gas_price)
            .expect("test gas_value should be computable");
        GasInfo {
            gas_value,
            gas_used,
            gas_price,
        }
    }

    #[test]
    fn actual_fee_height_inactive_at_genesis_block() {
        assert!(!is_actual_fee_projection_height_active(0));
    }

    #[test]
    fn actual_fee_height_inactive_at_activation_height() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
        assert!(!is_actual_fee_projection_height_active(activation_height));
    }

    #[test]
    fn actual_fee_height_active_after_activation_height() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
        let first_active_block = activation_height
            .checked_add(1)
            .expect("activation height must be strictly below u64::MAX");
        assert!(is_actual_fee_projection_height_active(first_active_block));
    }

    #[test]
    fn derive_receipt_gas_used_from_actual_fee_uses_uniform_price() {
        let gas_info = gas_info(
            [21_000, 0],
            [sov_bank::Amount::new(10), sov_bank::Amount::new(10)],
        );
        let gas_used = derive_receipt_gas_used_from_actual_fee(&gas_info)
            .expect("uniform gas price should derive exact gas");

        assert_eq!(gas_used, 21_000);
    }

    #[test]
    fn derive_receipt_gas_used_from_actual_fee_rounds_up_for_non_uniform_prices() {
        // actual fee = 7*10 + 5*11 = 125, primary price = 10, projected gas = ceil(12.5) = 13.
        let gas_info = gas_info(
            [7, 5],
            [sov_bank::Amount::new(10), sov_bank::Amount::new(11)],
        );
        let gas_used = derive_receipt_gas_used_from_actual_fee(&gas_info)
            .expect("non-uniform prices should project with upward rounding");

        assert_eq!(gas_used, 13);
    }

    #[test]
    fn derive_receipt_gas_used_from_actual_fee_rejects_zero_price() {
        let gas_info = gas_info(
            [21_000, 21_000],
            [sov_bank::Amount::ZERO, sov_bank::Amount::ZERO],
        );
        let err = derive_receipt_gas_used_from_actual_fee(&gas_info)
            .expect_err("zero gas price should fail hard");

        assert!(err.to_string().contains("zero primary gas price"));
    }

    #[test]
    fn derive_receipt_gas_used_from_actual_fee_rejects_u64_overflow() {
        let gas_info: GasInfo<GasUnit<2>> = GasInfo {
            gas_value: sov_bank::Amount::MAX,
            gas_used: [0, 0].into(),
            gas_price: [sov_bank::Amount::new(1), sov_bank::Amount::new(1)].into(),
        };
        let err = derive_receipt_gas_used_from_actual_fee(&gas_info)
            .expect_err("gas_used projection should fail when it does not fit in u64");

        assert!(err.to_string().contains("does not fit in u64"));
    }
}
