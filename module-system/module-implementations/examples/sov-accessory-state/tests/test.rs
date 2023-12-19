#![cfg(feature = "native")]

use sov_accessory_state::{AccessorySetter, CallMessage};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::prelude::*;
use sov_modules_api::{Address, Context, Module, WorkingSet};
use sov_prover_storage_manager::new_orphan_storage;
use sov_state::Storage;

#[test]
/// Check that:
/// 1. Accessory state does not change normal state root hash
/// 2. Accessory state is saved to underlying the database
fn test_accessory_value_setter() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    let mut working_set_for_state = WorkingSet::new(storage.clone());
    let mut working_set_for_accessory = WorkingSet::new(storage.clone());
    let mut working_set_for_check: WorkingSet<DefaultContext> = WorkingSet::new(storage.clone());

    let admin = Address::from([1; 32]);
    let sequencer = Address::from([2; 32]);
    let context = DefaultContext::new(admin, sequencer, 1);

    let module = AccessorySetter::<DefaultContext>::default();

    module.genesis(&(), &mut working_set_for_state).unwrap();
    module
        .call(
            CallMessage::SetValue("FooBar".to_string()),
            &context,
            &mut working_set_for_state,
        )
        .unwrap();

    let (reads_writes, witness) = working_set_for_state.checkpoint().freeze();
    let state_root_hash = storage.validate_and_commit(reads_writes, &witness).unwrap();

    module
        .call(
            CallMessage::SetValueAccessory("FooBar".to_string()),
            &context,
            &mut working_set_for_accessory,
        )
        .unwrap();

    let mut checkpoint = working_set_for_accessory.checkpoint();
    let (reads_writes, witness) = checkpoint.freeze();
    let accessory_writes = checkpoint.freeze_non_provable();
    let state_root_hash_2 = storage
        .validate_and_commit_with_accessory_update(reads_writes, &witness, &accessory_writes)
        .unwrap();

    assert_eq!(
        Some("FooBar".to_string()),
        module.state_value.get(&mut working_set_for_check),
        "Provable state has not been propagated to the underlying storage!"
    );
    assert_eq!(
        Some("FooBar".to_string()),
        module
            .accessory_value
            .get(&mut working_set_for_check.accessory_state()),
        "Accessory state has not been propagated to the underlying storage!"
    );

    assert_eq!(
        state_root_hash, state_root_hash_2,
        "Accessory update has affected the state root hash!"
    );
}
