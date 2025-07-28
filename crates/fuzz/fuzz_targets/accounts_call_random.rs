#![no_main]

use libfuzzer_sys::arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use sov_accounts::{Accounts, CallMessage};
use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::{Context, Module, StateCheckpoint, WorkingSet};
use sov_test_utils::storage::SimpleStorageManager;

type S = sov_test_utils::TestSpec;

// Check arbitrary, random calls
fuzz_target!(|input: (&[u8], Vec<(Context<S>, CallMessage)>)| {
    let storage_manager = SimpleStorageManager::new();
    let storage = storage_manager.create_storage();
    let mut state = StateCheckpoint::new(storage, &MockKernel::<S>::default());

    let (seed, msgs) = input;
    let u = &mut Unstructured::new(seed);
    let maybe_accounts = Accounts::arbitrary_workset(u, &mut state).unwrap();

    let mut accounts: Accounts<S> = maybe_accounts;
    let mut working_set: WorkingSet<S> = state.to_working_set_unmetered();

    for (ctx, msg) in msgs {
        // assert malformed calls won't panic
        accounts.call(msg, &ctx, &mut working_set).ok();
    }
});
