use borsh::BorshSerialize;
use sov_default_stf::{AppTemplate, Batch, SequencerOutcome, TxEffect};
use sov_modules_api::{
    default_context::DefaultContext, default_signature::private_key::DefaultPrivateKey, Address,
};
use sov_rollup_interface::{mocks::MockZkvm, stf::BatchReceipt};
use sov_state::ProverStorage;
use std::path::Path;

use crate::{
    app::DemoApp,
    genesis_config::{
        create_demo_genesis_config, generate_address, DEMO_SEQUENCER_DA_ADDRESS,
        DEMO_SEQ_PUB_KEY_STR,
    },
    runtime::{GenesisConfig, Runtime},
    tx_hooks_impl::DemoAppTxHooks,
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

pub fn create_demo_config(
    initial_sequencer_balance: u64,
    value_setter_admin_private_key: &DefaultPrivateKey,
    election_admin_private_key: &DefaultPrivateKey,
) -> GenesisConfig<DefaultContext> {
    create_demo_genesis_config::<DefaultContext>(
        initial_sequencer_balance,
        generate_address::<DefaultContext>(DEMO_SEQ_PUB_KEY_STR),
        DEMO_SEQUENCER_DA_ADDRESS.to_vec(),
        value_setter_admin_private_key,
        election_admin_private_key,
    )
}

pub fn events_count(apply_blob_outcome: &BatchReceipt<SequencerOutcome, TxEffect>) -> usize {
    let events = apply_blob_outcome
        .tx_receipts
        .iter()
        .flat_map(|receipts| receipts.events.iter());

    for e in events.clone() {
        println!("{:?}", e.try_to_string());
    }

    events.count()
}
