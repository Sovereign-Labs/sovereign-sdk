//! Tests for the state API.
//! Note: these tests should be moved back inside the source folder the `sov-modules-api` crate
//! as it directly uses structs that should be hidden from the public API.

mod archival;
mod compute_state_update;
mod namespaces;
mod structs;

use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::{
    execution_mode, CryptoSpec, KernelStateValue, Spec, StateCheckpoint, Storage,
};
use sov_test_utils::storage::{ForklessStorageManager, SimpleStorageManager};
use sov_test_utils::{validate_and_materialize, TestSpec};
use unwrap_infallible::UnwrapInfallible;

pub type Zk =
    sov_modules_api::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, execution_mode::Zk>;
pub type TestHasher = <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher;
pub type StorageSpec = sov_state::DefaultStorageSpec<TestHasher>;

pub fn commit_to_storage<S, Sm>(
    state: StateCheckpoint<S>,
    storage: S::Storage,
    kernel: &mut MockKernel<S>,
    storage_manager: &mut Sm,
    pre_state_root: <<S as Spec>::Storage as Storage>::Root,
) where
    S: Spec,
    Sm: ForklessStorageManager<Storage = S::Storage>,
{
    let (cache_log, _, witness) = state.freeze();

    let (root_hash, state_update) = storage
        .compute_state_update(cache_log, &witness, pre_state_root)
        .expect("Compute state update must succeed");
    storage_manager.commit_state_update(storage, state_update, root_hash);

    kernel.increase_heights();
}

fn increase_value_and_commit<S, Sm>(
    state_value: &mut KernelStateValue<u32>,
    storage: S::Storage,
    kernel: &mut MockKernel<S>,
    storage_manager: &mut Sm,
    pre_state_root: <<S as Spec>::Storage as Storage>::Root,
) where
    S: Spec,
    Sm: ForklessStorageManager<Storage = S::Storage>,
{
    let mut state: StateCheckpoint<S> = StateCheckpoint::new(storage.clone(), kernel);

    // Setting value, starting from 0
    let value = match state_value.get(&mut state).unwrap_infallible() {
        None => 0,
        Some(past) => past + 1,
    };

    state_value.set(&value, &mut state).unwrap_infallible();

    commit_to_storage(state, storage, kernel, storage_manager, pre_state_root);
}
