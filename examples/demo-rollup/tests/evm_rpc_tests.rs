#![allow(dead_code)]

#[path = "test_helpers.rs"]
mod test_helpers;

#[path = "evm"]
mod evm {
    pub(crate) mod evm_test_helper;

    mod evm_rpc;
    mod evm_rpc_compliance_validation;
    mod evm_rpc_compliance_validation_2;
}
