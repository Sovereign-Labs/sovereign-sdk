use sov_modules_api::macros::config_value;

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
}
