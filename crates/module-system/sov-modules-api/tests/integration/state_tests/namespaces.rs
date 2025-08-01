use std::convert::Infallible;

use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::capabilities::Kernel;
use sov_modules_api::{
    KernelStateValue, Spec, StateCheckpoint, StateMap, StateValue, VersionedStateValue,
};
use sov_state::{BorshCodec, Prefix, ProvableNamespace, StateRoot};
use sov_test_utils::storage::{SimpleNomtStorageManager, SimpleStorageManager};
use sov_test_utils::{TestNomtSpec, TestSpec};

use crate::state_tests::{commit_to_storage, ForklessStorageManager};

#[test]
fn test_jmt_state_value_user_namespace() -> Result<(), Infallible> {
    let mut storage_manager = SimpleStorageManager::new();
    storage_manager.genesis();
    test_state_value_user_namespace::<TestSpec, _>(storage_manager)
}

// TODO: Do we want to unify these 2 tests? The only differ by value passed. Probably
/// Test that the state values with a standard working set get written to the user space
fn test_state_value_user_namespace<S, Sm>(mut storage_manager: Sm) -> Result<(), Infallible>
where
    S: Spec,
    Sm: ForklessStorageManager<Storage = S::Storage>,
{
    let (storage, prev_root) = storage_manager.create_storage_with_root();

    let mut state_value = StateValue::with_codec(Prefix::new(vec![0]), BorshCodec);

    let mut kernel = MockKernel::<S>::default();

    // Native execution
    let mut state: StateCheckpoint<S> = StateCheckpoint::new(storage.clone(), &kernel);
    state_value.set(&11, &mut state)?;
    commit_to_storage(state, storage, &mut kernel, &mut storage_manager, prev_root);

    let (storage, root) = storage_manager.create_storage_with_root();

    // In the first version, the user and the kernel root hashes are different
    let kernel_root_hash = root.namespace_root(ProvableNamespace::Kernel);
    let user_root_hash = root.namespace_root(ProvableNamespace::User);
    assert_ne!(kernel_root_hash, user_root_hash);

    let mut state: StateCheckpoint<S> = StateCheckpoint::new(storage.clone(), &kernel);
    let _ = state_value.get(&mut state);
    state_value.set(&22, &mut state)?;
    commit_to_storage(state, storage, &mut kernel, &mut storage_manager, root);
    let new_root = storage_manager.current_root();

    // Then the kernel is the same but the user root hash changes
    let new_kernel_root_hash = new_root.namespace_root(ProvableNamespace::Kernel);
    let new_user_root_hash = new_root.namespace_root(ProvableNamespace::User);
    assert_eq!(kernel_root_hash, new_kernel_root_hash);
    assert_ne!(new_kernel_root_hash, new_user_root_hash);

    Ok(())
}

#[test]
fn test_jmt_state_value_kernel_namespace() -> Result<(), Infallible> {
    let storage_manager = SimpleStorageManager::new();
    test_state_value_kernel_namespace::<TestSpec, _>(storage_manager)
}

#[test]
fn test_nomt_state_value_kernel_namespace() -> Result<(), Infallible> {
    let storage_manager = SimpleNomtStorageManager::new();
    test_state_value_kernel_namespace::<TestNomtSpec, _>(storage_manager)
}

/// Test that the state values with a kernel working set get written to the kernel space
fn test_state_value_kernel_namespace<S, Sm>(mut storage_manager: Sm) -> Result<(), Infallible>
where
    S: Spec,
    Sm: ForklessStorageManager<Storage = S::Storage>,
{
    let (storage, prev_root) = storage_manager.create_storage_with_root();

    let mut kernel = MockKernel::<S>::default();

    let mut state_value = KernelStateValue::with_codec(Prefix::new(vec![0]), BorshCodec);

    // Native execution
    let mut state: StateCheckpoint<S> = StateCheckpoint::new(storage.clone(), &kernel);
    let mut kernel_working_set = kernel.accessor(&mut state);
    state_value.set(&11, &mut kernel_working_set)?;

    commit_to_storage(state, storage, &mut kernel, &mut storage_manager, prev_root);
    let (storage, root) = storage_manager.create_storage_with_root();

    // In the first version the user and the kernel root hashes are different
    let kernel_root_hash = root.namespace_root(ProvableNamespace::Kernel);
    let user_root_hash = root.namespace_root(ProvableNamespace::User);
    assert_ne!(kernel_root_hash, user_root_hash);

    let mut state: StateCheckpoint<S> = StateCheckpoint::new(storage.clone(), &kernel);
    let mut kernel_working_set = kernel.accessor(&mut state);
    let _ = state_value.get(&mut kernel_working_set);
    state_value.set(&22, &mut kernel_working_set)?;
    commit_to_storage(state, storage, &mut kernel, &mut storage_manager, root);

    let new_root = storage_manager.current_root();
    // Then the user is the same but the user root hash changes
    let new_kernel_root_hash = new_root.namespace_root(ProvableNamespace::Kernel);
    let new_user_root_hash = new_root.namespace_root(ProvableNamespace::User);
    assert_eq!(user_root_hash, new_user_root_hash);
    assert_ne!(new_kernel_root_hash, new_user_root_hash);

    Ok(())
}

#[test]
fn test_jmt_state_map_user_namespace() -> Result<(), Infallible> {
    let storage_manager = SimpleStorageManager::new();
    test_state_map_user_namespace::<TestSpec, _>(storage_manager)
}

/// Test that the state maps with a standard working set get written to the user space
fn test_state_map_user_namespace<S, Sm>(mut storage_manager: Sm) -> Result<(), Infallible>
where
    S: Spec,
    Sm: ForklessStorageManager<Storage = S::Storage>,
{
    let (storage, prev_root) = storage_manager.create_storage_with_root();

    let mut state_value = StateMap::with_codec(Prefix::new(vec![0]), BorshCodec);
    let mut kernel = MockKernel::<S>::default();

    // Native execution
    let mut state: StateCheckpoint<S> = StateCheckpoint::new(storage.clone(), &kernel);
    state_value.set(&11, &0, &mut state)?;

    // Committing data at height 0
    commit_to_storage(state, storage, &mut kernel, &mut storage_manager, prev_root);
    let (storage, root) = storage_manager.create_storage_with_root();

    let kernel_root_hash = root.namespace_root(ProvableNamespace::Kernel);
    let user_root_hash = root.namespace_root(ProvableNamespace::User);
    // In the first version the user and the kernel root hashes are different
    assert_ne!(kernel_root_hash, user_root_hash);

    let mut state: StateCheckpoint<S> = StateCheckpoint::new(storage.clone(), &kernel);
    state_value.set(&11, &0, &mut state)?;
    let _ = state_value.get(&0, &mut state);
    state_value.set(&22, &0, &mut state)?;
    // Committing at height = 1
    commit_to_storage(state, storage, &mut kernel, &mut storage_manager, root);
    let new_root = storage_manager.current_root();

    // Then the kernel is the same but the user root hash changes
    let new_kernel_root_hash = new_root.namespace_root(ProvableNamespace::Kernel);
    let new_user_root_hash = new_root.namespace_root(ProvableNamespace::User);
    assert_eq!(kernel_root_hash, new_kernel_root_hash);
    assert_ne!(user_root_hash, new_user_root_hash);

    Ok(())
}

#[test]
fn test_jmt_versioned_state_value_kernel_namespace() -> Result<(), Infallible> {
    let storage_manager = SimpleStorageManager::new();
    test_versioned_state_value_kernel_namespace::<TestSpec, _>(storage_manager)
}

#[test]
fn test_nomt_versioned_state_value_kernel_namespace() -> Result<(), Infallible> {
    let storage_manager = SimpleNomtStorageManager::new();
    test_versioned_state_value_kernel_namespace::<TestNomtSpec, _>(storage_manager)
}

/// Test that the kernel state maps with a kernel working set get written to the kernel space
fn test_versioned_state_value_kernel_namespace<S, Sm>(
    mut storage_manager: Sm,
) -> Result<(), Infallible>
where
    S: Spec,
    Sm: ForklessStorageManager<Storage = S::Storage>,
{
    let (storage, prev_root) = storage_manager.create_storage_with_root();

    let mut state_value = VersionedStateValue::with_codec(Prefix::new(vec![0]), BorshCodec);

    let mut kernel = MockKernel::<S>::default();

    // Native execution
    let mut state = StateCheckpoint::new(storage.clone(), &kernel);
    let mut kernel_working_set = kernel.accessor(&mut state);
    state_value
        .set_true_current(&11, &mut kernel_working_set)
        .unwrap();

    commit_to_storage(state, storage, &mut kernel, &mut storage_manager, prev_root);
    let (storage, root) = storage_manager.create_storage_with_root();

    // In the first version the user and the kernel root hashes are different from one another
    let kernel_root_hash = root.namespace_root(ProvableNamespace::Kernel);
    let user_root_hash = root.namespace_root(ProvableNamespace::User);
    assert_ne!(kernel_root_hash, user_root_hash);

    let mut state = StateCheckpoint::new(storage.clone(), &kernel);
    let mut kernel_working_set = kernel.accessor(&mut state);
    let _ = state_value.get_current(&mut kernel_working_set);
    state_value
        .set_true_current(&22, &mut kernel_working_set)
        .unwrap();
    commit_to_storage(state, storage, &mut kernel, &mut storage_manager, root);
    let (storage, new_root) = storage_manager.create_storage_with_root();
    let new_kernel_root_hash = new_root.namespace_root(ProvableNamespace::Kernel);
    let new_user_root_hash = new_root.namespace_root(ProvableNamespace::User);

    // Then the kernel is the same but the user root hash changes
    assert_eq!(user_root_hash, new_user_root_hash);
    assert_ne!(new_kernel_root_hash, new_user_root_hash);

    // Check that we can get the current value with a standard working set
    let mut kernel_reset = MockKernel::<S>::default();
    let mut state = StateCheckpoint::new(storage.clone(), &kernel_reset);
    let val_0 = state_value
        .get_current(&mut state)?
        .expect("We should be able to retrieve the state value");
    assert_eq!(val_0, 11);

    kernel_reset.increase_heights();

    let mut state = StateCheckpoint::new(storage.clone(), &kernel_reset);
    let val_0 = state_value
        .get_current(&mut state)?
        .expect("We should be able to retrieve the state value");
    assert_eq!(val_0, 22);

    Ok(())
}
