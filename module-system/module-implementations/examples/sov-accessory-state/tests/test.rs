#![cfg(feature = "native")]

use sov_accessory_state::{AccessorySetter, CallMessage};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Context, Module, WorkingSet};
use sov_state::{ProverStorage, Storage};

#[test]
fn test_value_setter() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage.clone());
    let mut working_set_2 = WorkingSet::new(storage.clone());
    let mut working_set_4: WorkingSet<DefaultContext> = WorkingSet::new(storage.clone());

    let admin = Address::from([1; 32]);
    let context = DefaultContext::new(admin);

    let module = AccessorySetter::<DefaultContext>::default();

    module.genesis(&(), &mut working_set).unwrap();
    module
        .call(
            CallMessage::SetValue("FooBar".to_string()),
            &context,
            &mut working_set,
        )
        .unwrap();

    let (reads_writes, witness) = working_set.checkpoint().freeze();
    let state_root_hash = storage.validate_and_commit(reads_writes, &witness).unwrap();

    module
        .call(
            CallMessage::SetValueAccessory("FooBar".to_string()),
            &context,
            &mut working_set_2,
        )
        .unwrap();

    let mut checkpoint = working_set_2.checkpoint();
    let (reads_writes, witness) = checkpoint.freeze();
    let accessory_writes = checkpoint.freeze_non_provable();
    let state_root_hash_2 = storage
        .validate_and_commit_with_accessory_update(reads_writes, &witness, &accessory_writes)
        .unwrap();

    assert_eq!(
        module.state_value.get(&mut working_set_4),
        Some("FooBar".to_string())
    );
    assert_eq!(
        module
            .accessory_value
            .get(&mut working_set_4.accessory_state()),
        Some("FooBar".to_string())
    );

    assert_eq!(state_root_hash, state_root_hash_2);
}
