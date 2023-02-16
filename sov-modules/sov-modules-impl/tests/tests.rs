use sov_modules_api::mocks::{MockContext, ZkMockContext};
use sov_modules_api::{Context, ModuleInfo, Prefix};
use sov_modules_macros::ModuleInfo;
use sov_state::storage::{StorageKey, StorageValue};
use sov_state::{JmtStorage, StateMap, StateValue, Storage, ZkStorage};

pub mod module_a {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct ModuleA<C: Context> {
        #[state]
        pub(crate) state_1_a: StateMap<String, String, C::Storage>,

        #[state]
        pub(crate) state_2_a: StateValue<String, C::Storage>,
    }

    impl<C: Context> ModuleA<C> {
        pub fn update(&mut self, key: &str, value: &str) {
            self.state_1_a.set(&key.to_owned(), value.to_owned());
            self.state_2_a.set(value.to_owned())
        }
    }
}

pub mod module_b {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct ModuleB<C: Context> {
        #[state]
        state_1_b: StateMap<String, String, C::Storage>,

        #[module]
        pub(crate) mod_1_a: module_a::ModuleA<C>,
    }

    impl<C: Context> ModuleB<C> {
        pub fn update(&mut self, key: &str, value: &str) {
            self.state_1_b.set(&key.to_owned(), value.to_owned());
            self.mod_1_a.update("key_from_b", value);
        }
    }
}

mod module_c {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct ModuleC<C: Context> {
        #[module]
        pub(crate) mod_1_a: module_a::ModuleA<C>,

        #[module]
        mod_1_b: module_b::ModuleB<C>,
    }

    impl<C: Context> ModuleC<C> {
        pub fn execute(&mut self, key: &str, value: &str) {
            self.mod_1_a.update(key, value);
            self.mod_1_b.update(key, value);
            self.mod_1_a.update(key, value);
        }
    }
}

#[test]
fn nested_module_call_test() {
    let native_storage = JmtStorage::default();

    // Test the `native` execution.
    {
        execute_module_logic::<MockContext>(native_storage.clone());
        test_state_update::<MockContext>(native_storage.clone());
    }

    // Test the `zk` execution.
    {
        let zk_storage = ZkStorage::new(native_storage.get_first_reads());
        execute_module_logic::<ZkMockContext>(zk_storage.clone());
        test_state_update::<ZkMockContext>(zk_storage);
    }
}

fn execute_module_logic<C: Context>(storage: C::Storage) {
    let module = &mut module_c::ModuleC::<C>::new(storage);
    module.execute("some_key", "some_value");
}

fn test_state_update<C: Context>(storage: C::Storage) {
    let module = <module_c::ModuleC<C> as ModuleInfo<C>>::new(storage.clone());

    let expected_value = StorageValue::new("some_value");

    {
        let prefix = Prefix::new("tests::module_a", "ModuleA", "state_1_a");
        let key = StorageKey::new(&prefix.into(), &"some_key");
        let value = storage.get(key).unwrap();

        assert_eq!(expected_value, value);
    }

    {
        let prefix = Prefix::new("tests::module_b", "ModuleB", "state_1_b");
        let key = StorageKey::new(&prefix.into(), &"some_key");
        let value = storage.get(key).unwrap();

        assert_eq!(expected_value, value);
    }

    {
        let prefix = Prefix::new("tests::module_a", "ModuleA", "state_1_a");
        let key = StorageKey::new(&prefix.into(), &"key_from_b");
        let value = storage.get(key).unwrap();

        assert_eq!(expected_value, value);
    }

    {
        let value = module.mod_1_a.state_2_a.get().unwrap();
        assert_eq!("some_value".to_owned(), value);
    }
}
