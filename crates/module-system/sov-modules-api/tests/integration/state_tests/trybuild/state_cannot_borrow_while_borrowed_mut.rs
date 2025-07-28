use sov_mock_zkvm::MockZkvm;
use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::{StateCheckpoint, StateValue};
use sov_rollup_interface::execution_mode::Native;
use sov_state::{BorshCodec, Prefix};
use sov_test_utils::storage::SimpleStorageManager;
use sov_test_utils::MockDaSpec;
use unwrap_infallible::UnwrapInfallible;

type TestSpec = sov_modules_api::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

fn main() {
    let storage_manager = SimpleStorageManager::new();
    let storage = storage_manager.create_storage();
    let kernel = MockKernel::<TestSpec>::new(4, 1);
    let mut state = StateCheckpoint::new(storage, &kernel);

    let prefix = Prefix::new(b"test".to_vec());
    let mut value = crate::StateValue::<RollupHeight>::with_codec(prefix.clone(), BorshCodec);
    value.set(&RollupHeight::new(100), &mut state);
    let val = value.borrow_mut(&mut state).unwrap_infallible().unwrap();
    value.borrow(&mut state).unwrap_infallible();
    assert_eq!(*val, RollupHeight::new(100));
}
