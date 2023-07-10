use std::path::Path;

use borsh::BorshSerialize;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::Address;
use sov_modules_stf_template::{AppTemplate, Batch, SequencerOutcome, TxEffect};
use sov_rollup_interface::stf::BatchReceipt;
use sov_state::ProverStorage;

use crate::app::DemoApp;
use crate::runtime::Runtime;

mod data_generation;
mod stf_tests;
mod tx_revert_tests;
pub(crate) type C = DefaultContext;

pub type TestBlob = sov_rollup_interface::mocks::TestBlob<Address>;

pub fn create_new_demo(
    path: impl AsRef<Path>,
) -> DemoApp<DefaultContext, sov_rollup_interface::mocks::MockZkvm> {
    let runtime = Runtime::default();
    let storage = ProverStorage::with_path(path).unwrap();
    AppTemplate::new(storage, runtime)
}

pub fn has_tx_events(apply_blob_outcome: &BatchReceipt<SequencerOutcome, TxEffect>) -> bool {
    let events = apply_blob_outcome
        .tx_receipts
        .iter()
        .flat_map(|receipts| receipts.events.iter());

    events.peekable().peek().is_some()
}

pub fn new_test_blob(batch: Batch, address: &[u8]) -> TestBlob {
    let address = Address::try_from(address).unwrap();
    let data = batch.try_to_vec().unwrap();
    TestBlob::new(data, address, [0; 32])
}
