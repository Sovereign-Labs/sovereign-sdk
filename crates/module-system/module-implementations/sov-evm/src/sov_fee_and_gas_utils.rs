use crate::Receipt;
use anyhow::{bail, ensure, Context};
use sov_modules_api::macros::config_value;
use sov_modules_api::{GasInfo, Spec};
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
    // Intentional fail-closed behavior: once projection is active, we reject transactions
    // whose charged gas dimensions cannot be reconciled into a single exact receipt gas value.
    let projected_gas_used =
        derive_receipt_gas_used_from_actual_fee(tx_fee_paid, gas_info.gas_price.as_ref())?;

    if projected_gas_used == current_gas_used {
        return Ok(None);
    }

    let previous_cumulative = receipt
        .receipt
        .cumulative_gas_used
        .checked_sub(current_gas_used)
        .context("EVM: receipt cumulative gas underflow while projecting exact fee")?;
    let projected_cumulative = previous_cumulative
        .checked_add(projected_gas_used)
        .context("EVM: receipt cumulative gas overflow while projecting exact fee")?;

    Ok(Some(ProjectedReceiptGas {
        gas_used: projected_gas_used,
        cumulative_gas_used: projected_cumulative,
    }))
}

fn derive_receipt_gas_used_from_actual_fee(
    tx_fee_paid: sov_bank::Amount,
    gas_price_per_dimension: &[sov_bank::Amount],
) -> anyhow::Result<u64> {
    let Some(uniform_gas_price) = gas_price_per_dimension.first() else {
        bail!("EVM: gas price vector must have at least one dimension");
    };

    ensure!(
        uniform_gas_price.0 > 0,
        "EVM: cannot reconcile receipt from actual fee with zero gas price"
    );
    // Intentional: non-uniform per-dimension prices are treated as a hard correctness error.
    // If we cannot derive one exact gas_used value, we reject instead of emitting mismatched fees.
    ensure!(
        gas_price_per_dimension
            .iter()
            .all(|price| price == uniform_gas_price),
        "EVM: cannot reconcile receipt from actual fee with non-uniform gas prices: {gas_price_per_dimension:?}"
    );

    ensure!(
        tx_fee_paid.0 % uniform_gas_price.0 == 0,
        "EVM: tx fee {tx_fee_paid} is not divisible by uniform gas price {uniform_gas_price}"
    );

    let projected_gas_used_u128 = tx_fee_paid
        .0
        .checked_div(uniform_gas_price.0)
        .expect("division by zero should be impossible because zero gas price is rejected above");
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let gas_used = derive_receipt_gas_used_from_actual_fee(
            sov_bank::Amount::new(210_000),
            &[sov_bank::Amount::new(10), sov_bank::Amount::new(10)],
        )
        .expect("uniform gas price should derive exact gas");

        assert_eq!(gas_used, 21_000);
    }

    #[test]
    fn derive_receipt_gas_used_from_actual_fee_rejects_non_uniform_prices() {
        let err = derive_receipt_gas_used_from_actual_fee(
            sov_bank::Amount::new(210_000),
            &[sov_bank::Amount::new(10), sov_bank::Amount::new(11)],
        )
        .expect_err("non-uniform gas prices should fail hard");

        assert!(err.to_string().contains("non-uniform gas prices"));
    }

    #[test]
    fn derive_receipt_gas_used_from_actual_fee_rejects_zero_price() {
        let err = derive_receipt_gas_used_from_actual_fee(
            sov_bank::Amount::new(210_000),
            &[sov_bank::Amount::ZERO, sov_bank::Amount::ZERO],
        )
        .expect_err("zero gas price should fail hard");

        assert!(err.to_string().contains("zero gas price"));
    }
}
