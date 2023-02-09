
use sov_modules_api::mocks::MockContext;
use sov_modules_api::{Context, Prefix};
use sov_modules_macros::ModuleInfo;
use sov_state::storage::{StorageKey, StorageValue};
use sov_state::{JmtStorage, SingletonKey, StateMap, StateValue, Storage};

pub mod module_a {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct ModuleA<C: Context> {
        #[state]
        state_1_a: StateMap<String, String, C::Storage>,

        #[state]
        state_2_a: StateValue<String, C::Storage>,
    }

    impl<C: Context> ModuleA<C> {
        pub fn update(&mut self, key: &str, value: &str) {
            self.state_1_a.set(key.to_owned(), value.to_owned());
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
        mod_1_a: module_a::ModuleA<C>,
    }

    impl<C: Context> ModuleB<C> {
        pub fn update(&mut self, key: &str, value: &str) {
            self.state_1_b.set(key.to_owned(), value.to_owned());
            self.mod_1_a.update("key_from_b", value);
        }
    }
}

mod module_c {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct ModuleC<C: Context> {
        #[module]
        mod_1_a: module_a::ModuleA<C>,

        #[module]
        mod_1_b: module_b::ModuleB<C>,
    }

    impl<C: Context> ModuleC<C> {
        pub fn update(&mut self, key: &str, value: &str) {
            self.mod_1_a.update(key, value);
            self.mod_1_b.update(key, value);
            self.mod_1_a.update(key, value);
        }
    }
}
#[test]
fn nested_module_call_test() {
    let test_storage = JmtStorage::default();
    let module = &mut module_c::ModuleC::<MockContext>::_new(test_storage.clone());
    module.update("some_key", "some_value");

    let expected_value = StorageValue::new("some_value");

    {
        let prefix = Prefix::new("tests::module_a", "ModuleA", "state_1_a");
        let key = StorageKey::new(&prefix.into(), "some_key");
        let value = test_storage.get(key).unwrap();

        assert_eq!(expected_value, value);
    }

    {
        let prefix = Prefix::new("tests::module_b", "ModuleB", "state_1_b");
        let key = StorageKey::new(&prefix.into(), "some_key");
        let value = test_storage.get(key).unwrap();

        assert_eq!(expected_value, value);
    }

    {
        let prefix = Prefix::new("tests::module_a", "ModuleA", "state_1_a");
        let key = StorageKey::new(&prefix.into(), "key_from_b");
        let value = test_storage.get(key).unwrap();

        assert_eq!(expected_value, value);
    }

    {
        let prefix = Prefix::new("tests::module_a", "ModuleA", "state_2_a");
        let key = StorageKey::new(&prefix.into(), SingletonKey);
        let value = test_storage.get(key).unwrap();

        assert_eq!(expected_value, value);
    }
}
