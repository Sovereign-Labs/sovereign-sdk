use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::{Context, Event, Prefix, StateMap, WorkingSet};
use sov_state::{ProverStorage, Storage, ZkStorage};

use super::helpers::module_c;

#[test]
fn nested_module_call_test() {
    let tmpdir = tempfile::tempdir().unwrap();
    let native_storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(native_storage.clone());

    // Test the `native` execution.
    {
        execute_module_logic::<DefaultContext>(&mut working_set);
        test_state_update::<DefaultContext>(&mut working_set);
    }
    assert_eq!(
        working_set.events(),
        &vec![
            Event::new("module C", "execute"),
            Event::new("module A", "update"),
            Event::new("module B", "update"),
            Event::new("module A", "update"),
            Event::new("module A", "update"),
        ]
    );

    let (log, witness) = working_set.checkpoint().freeze();
    native_storage
        .validate_and_commit(log, &witness)
        .expect("State update is valid");

    // Test the `zk` execution.
    {
        let zk_storage = ZkStorage::new();
        let working_set = &mut WorkingSet::with_witness(zk_storage, witness);
        execute_module_logic::<ZkDefaultContext>(working_set);
        test_state_update::<ZkDefaultContext>(working_set);
    }
}

fn execute_module_logic<C: Context>(working_set: &mut WorkingSet<C>) {
    let module = &mut module_c::ModuleC::<C>::default();
    module.execute("some_key", "some_value", working_set);
}

fn test_state_update<C: Context>(working_set: &mut WorkingSet<C>) {
    let module = <module_c::ModuleC<C> as Default>::default();

    let expected_value = "some_value".to_owned();

    {
        let prefix = Prefix::new_storage(
            "integration_tests::nested_modules::helpers::module_a",
            "ModuleA",
            "state_1_a",
        );
        let state_map = StateMap::<String, String>::new(prefix.into());
        let value = state_map.get(&"some_key".to_owned(), working_set).unwrap();

        assert_eq!(expected_value, value);
    }

    {
        let prefix = Prefix::new_storage(
            "integration_tests::nested_modules::helpers::module_b",
            "ModuleB",
            "state_1_b",
        );
        let state_map = StateMap::<String, String>::new(prefix.into());
        let value = state_map.get(&"some_key".to_owned(), working_set).unwrap();

        assert_eq!(expected_value, value);
    }

    {
        let prefix = Prefix::new_storage(
            "integration_tests::nested_modules::helpers::module_a",
            "ModuleA",
            "state_1_a",
        );
        let state_map = StateMap::<String, String>::new(prefix.into());
        let value = state_map.get(&"some_key".to_owned(), working_set).unwrap();

        assert_eq!(expected_value, value);
    }

    {
        let value = module.mod_1_a.state_2_a.get(working_set).unwrap();
        assert_eq!(expected_value, value);
    }
}
