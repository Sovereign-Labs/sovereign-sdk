use sov_modules_api::mocks::{MockContext, MockStorage};
use sov_modules_api::{Context, Prefix};
use sov_modules_macros::ModuleInfo;
use sov_state::storage::{StorageKey, StorageValue};
use sov_state::{StateMap, Storage};
use sovereign_sdk::serial::{Decode, Encode};

pub mod module_a {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct ModuleA<C: Context> {
        #[state]
        state_1_a: StateMap<String, String, C::Storage>,
    }

    impl<C: Context> ModuleA<C> {
        pub fn update(&self, key: &str, value: &str) {
            self.state_1_a.set(key.to_owned(), value.to_owned())
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
        pub fn update(&self, key: &str, value: &str) {
            self.state_1_b.set(key.to_owned(), value.to_owned());
            self.mod_1_a.update("insert_from_c", value);
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
        pub fn update(&self, key: &str, value: &str) {
            self.mod_1_a.update(key, value);
            self.mod_1_b.update(key, value);
            self.mod_1_a.update(key, value);
        }
    }
}

#[test]
fn test() {
    let test_storage = MockStorage::default();
    let module = &mut module_c::ModuleC::<MockContext>::_new(test_storage.clone());

    module.update("key", "some_value");

    //  "".encode(target)
    //let s = String::decode("some_value")
    let value = StorageValue::from("some_value");

    {
        let prefix = Prefix::new(
            "tests::module_a".to_owned(),
            "ModuleA".to_owned(),
            "state_1_a".to_owned(),
        );

        let key = StorageKey::new(&prefix.into(), "key");
        let v = test_storage.get(key).unwrap();
        assert_eq!(value, v);
    }

    {
        let prefix = Prefix::new(
            "tests::module_b".to_owned(),
            "ModuleB".to_owned(),
            "state_1_b".to_owned(),
        );

        let key = StorageKey::new(&prefix.into(), "key");
        let v = test_storage.get(key).unwrap();
        assert_eq!(value, v);
    }

    {
        let prefix = Prefix::new(
            "tests::module_a".to_owned(),
            "ModuleA".to_owned(),
            "state_1_a".to_owned(),
        );

        let key = StorageKey::new(&prefix.into(), "insert_from_c");
        let v = test_storage.get(key).unwrap();
        assert_eq!(value, v);
    }
}
