use std::convert::Infallible;
use std::sync::Arc;

use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::{ApiStateAccessor, KernelStateValue, StateCheckpoint};
use sov_state::{BorshCodec, Prefix};
use sov_test_utils::storage::{SimpleNomtStorageManager, SimpleStorageManager};
use sov_test_utils::TestNomtSpec;

use crate::state_tests::*;

#[test]
fn test_jmt_archival_state_updates_correctly() -> Result<(), Infallible> {
    let storage_manager = SimpleStorageManager::new();
    archival_state_updates_correctly::<TestSpec, _>(storage_manager)
}

#[test]
fn test_nomt_archival_state_updates_correctly() -> Result<(), Infallible> {
    let storage_manager = SimpleNomtStorageManager::new();
    archival_state_updates_correctly::<TestNomtSpec, _>(storage_manager)
}

/// Tests that the archival state is correctly retrieved from the DB and updates to the head state don't interfere
fn archival_state_updates_correctly<S, Sm>(mut storage_manager: Sm) -> Result<(), Infallible>
where
    S: Spec,
    Sm: ForklessStorageManager<Storage = S::Storage>,
{
    let mut kernel = MockKernel::<S>::default();
    let mut state_value = KernelStateValue::with_codec(Prefix::new(vec![0]), BorshCodec);

    for current_height in 0..100 {
        let (storage, prev_root) = storage_manager.create_storage_with_root();
        let state_checkpoint = StateCheckpoint::new(storage.clone(), &kernel);
        let api_accessor = ApiStateAccessor::new(&state_checkpoint, Arc::new(kernel.clone()));

        for past_height in 0..current_height {
            let mut archival_api_accessor = api_accessor
                .get_archival_state(RollupHeight::new(past_height))
                .unwrap();

            let value = state_value.get(&mut archival_api_accessor)?;

            assert_eq!(value, Some(past_height as u32));
        }

        increase_value_and_commit(
            &mut state_value,
            storage,
            &mut kernel,
            &mut storage_manager,
            prev_root,
        );
        let storage = storage_manager.create_prover_storage();
        let state_checkpoint = StateCheckpoint::new(storage, &kernel);
        let api_accessor = ApiStateAccessor::new(&state_checkpoint, Arc::new(kernel.clone()));

        for another_past_height in 0..=current_height {
            let mut archival_api_accessor = api_accessor
                .get_archival_state(RollupHeight::new(another_past_height))
                .unwrap();

            let value = state_value.get(&mut archival_api_accessor)?;

            assert_eq!(
                value,
                Some(another_past_height as u32),
                "Failed to get past value from {another_past_height} on height {current_height}"
            );
        }
    }

    Ok(())
}
