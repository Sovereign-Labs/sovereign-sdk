use sov_bank::Amount;
use sov_modules_api::macros::config_value;

/// Returns true once actual-fee projection is enabled for `block_number`.
pub(crate) fn is_actual_fee_projection_height_active(block_number: u64) -> bool {
    let apply_actual_fee_after_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
    block_number > apply_actual_fee_after_height
}

/// Returns true when actual-fee projection should be applied.
pub(crate) fn should_project_from_actual_fee(
    block_number: u64,
    fee_paid: Option<Amount>,
    gas_used: u64,
) -> bool {
    is_actual_fee_projection_height_active(block_number)
        && fee_paid.is_some_and(|fee| fee != Amount::ZERO)
        && gas_used > 0
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
    fn projection_guard_rejects_pre_activation_blocks() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
        let pre_activation = activation_height.saturating_sub(1);
        assert!(!should_project_from_actual_fee(
            pre_activation,
            Some(Amount::new(1)),
            1
        ));
    }

    #[test]
    fn projection_guard_rejects_missing_fee_paid() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
        let active_block = activation_height
            .checked_add(1)
            .expect("activation height must be strictly below u64::MAX");
        assert!(!should_project_from_actual_fee(active_block, None, 1));
    }

    #[test]
    fn projection_guard_rejects_zero_fee_paid() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
        let active_block = activation_height
            .checked_add(1)
            .expect("activation height must be strictly below u64::MAX");
        assert!(!should_project_from_actual_fee(
            active_block,
            Some(Amount::ZERO),
            1
        ));
    }

    #[test]
    fn projection_guard_rejects_zero_gas_used() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
        let active_block = activation_height
            .checked_add(1)
            .expect("activation height must be strictly below u64::MAX");
        assert!(!should_project_from_actual_fee(
            active_block,
            Some(Amount::new(1)),
            0
        ));
    }

    #[test]
    fn projection_guard_accepts_active_height_non_zero_fee_and_gas() {
        let activation_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
        let active_block = activation_height
            .checked_add(1)
            .expect("activation height must be strictly below u64::MAX");
        assert!(should_project_from_actual_fee(
            active_block,
            Some(Amount::new(1)),
            1
        ));
    }
}
