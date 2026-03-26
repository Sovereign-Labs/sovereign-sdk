#![allow(dead_code)]

#[path = "test_helpers.rs"]
mod test_helpers;

#[path = "evm"]
mod evm {
    pub(crate) mod evm_test_helper;

    mod evm_basefee;
    mod evm_call_fee_fields;
    mod evm_effective_gas_price;
    mod evm_fee_history;
    mod evm_gas_estimation;
    mod evm_max_fee_validation;
}
