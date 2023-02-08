use std::sync::Arc;

use sov_modules_api::mocks::{MockContext, MockStorage};
use sov_modules_api::Context;
use sov_modules_macros::ModuleInfo;
use sov_state::storage::{StorageKey, StorageValue};
use sov_state::{StateMap, Storage};
use sovereign_sdk::serial::Decode;

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
            //   self.state_1_b.set(key.to_owned(), value.to_owned());
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
            //    self.mod_1_a.update(key, value);
        }
    }
}

#[test]
fn test() {
    let test_storage = MockStorage::default();
    let module = &mut module_c::ModuleC::<MockContext>::_new(test_storage.clone());

    module.update("key", "some_value");

    for (k, v) in test_storage.storage.borrow().iter() {
        let mut ll: &[u8] = k;

        let key = String::decode(&mut ll);

        println!("{:?}", key);

        //  let mut ll: &[u8] = v;
        //  let x = String::decode(&mut ll);
        //  println!("{:?}", x);
    }

    let key = StorageKey::from("tests::module_a/ModuleA/state_1_a/key");

    let v = test_storage.get(key);
    println!("{:?}", v);
    /*
    let key1 = StorageKey {
        key: Arc::new("tests::module_a/ModuleA/state_1_a/key".as_bytes().to_vec()),
    };

    let value = StorageValue {
        value: Arc::new("some_value".as_bytes().to_vec()),
    };

    let g = test_storage.get(key1);
    let mut g = g.clone().unwrap();
    let v: Arc<Vec<u8>> = g.value;

    let mut ll: &[u8] = &v;

    let x = String::decode(&mut ll);

    println!("{:?}", x);

    let yy = "/";

    println!("{:?}", yy.as_bytes());

    */
}
