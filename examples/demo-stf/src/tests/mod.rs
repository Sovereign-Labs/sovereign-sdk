use borsh::BorshSerialize;
use sov_app_template::{AppTemplate, Batch};
use sov_modules_api::{default_context::DefaultContext, Address};
use sov_state::ProverStorage;
use std::path::Path;

use crate::{
    app::DemoApp, runtime::Runtime, tx_hooks_impl::DemoAppTxHooks,
    tx_verifier_impl::DemoAppTxVerifier,
};

mod data_generation;
mod stf_tests;
mod tx_revert_tests;
pub(crate) type C = DefaultContext;

pub type TestBlob = sov_rollup_interface::mocks::TestBlob<Address>;

pub fn new_test_blob(batch: Batch, address: &[u8]) -> TestBlob {
    let address = Address::try_from(address).unwrap();
    let data = batch.try_to_vec().unwrap();
    TestBlob::new(data, address)
}

pub fn create_new_demo(
    path: impl AsRef<Path>,
) -> DemoApp<DefaultContext, sov_rollup_interface::mocks::MockZkvm> {
    let runtime = Runtime::new();
    let storage = ProverStorage::with_path(path).unwrap();
    let tx_hooks = DemoAppTxHooks::new();
    let tx_verifier = DemoAppTxVerifier::new();
    AppTemplate::new(storage, runtime, tx_verifier, tx_hooks)
}
