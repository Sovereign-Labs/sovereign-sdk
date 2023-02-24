use sov_modules_api::{mocks::MockContext, CallResponse, Context, Error, Module, ModuleInfo};
use sov_modules_macros::{Genesis, ModuleInfo};
use sov_state::StateValue;
use sovereign_db::state_db::StateDB;

pub mod first_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct FirstTestStruct<C: Context> {
        #[state]
        pub state_in_first_struct: StateValue<u32, C::Storage>,
    }

    impl<C: Context> Module for FirstTestStruct<C> {
        type Context = C;
        type CallMessage = ();
        type QueryMessage = ();

        fn genesis(&mut self) -> Result<(), Error> {
            self.state_in_first_struct.set(1);
            Ok(())
        }

        fn call(
            &mut self,
            _msg: Self::CallMessage,
            _context: &Self::Context,
        ) -> Result<CallResponse, Error> {
            todo!()
        }

        fn query(&self, _msg: Self::QueryMessage) -> sov_modules_api::QueryResponse {
            todo!()
        }
    }

    impl<C: Context> FirstTestStruct<C> {
        pub(crate) fn get_state_value(&self) -> Option<u32> {
            self.state_in_first_struct.get()
        }
    }
}

pub mod second_test_module {
    use super::*;

    #[derive(borsh::BorshDeserialize, borsh::BorshSerialize)]
    pub struct TestType {}

    #[derive(ModuleInfo)]
    pub(crate) struct SecondTestStruct<C: Context> {
        #[state]
        pub state_in_second_struct: StateValue<u32, C::Storage>,
    }

    impl<C: Context> Module for SecondTestStruct<C> {
        type Context = C;
        type CallMessage = TestType;
        type QueryMessage = TestType;

        fn genesis(&mut self) -> Result<(), Error> {
            self.state_in_second_struct.set(2);
            Ok(())
        }

        fn call(
            &mut self,
            _msg: Self::CallMessage,
            _context: &Self::Context,
        ) -> Result<CallResponse, Error> {
            todo!()
        }

        fn query(&self, _msg: Self::QueryMessage) -> sov_modules_api::QueryResponse {
            todo!()
        }
    }

    impl<C: Context> SecondTestStruct<C> {
        pub(crate) fn get_state_value(&self) -> Option<u32> {
            self.state_in_second_struct.get()
        }
    }
}

// Debugging hint: To expand the macro in tests run: `cargo expand --test tests`
#[derive(Genesis)]
struct Runtime<C: Context> {
    _first: first_test_module::FirstTestStruct<C>,
    _second: second_test_module::SecondTestStruct<C>,
}

fn main() {
    use sov_modules_api::Genesis;

    type C = MockContext;
    let db = StateDB::temporary();
    let storage = Runtime::<C>::genesis(db).unwrap();

    let first_module = first_test_module::FirstTestStruct::<C>::new(storage.clone());
    let state_value = first_module.get_state_value();
    assert_eq!(state_value, Some(1));

    let second_module = second_test_module::SecondTestStruct::<C>::new(storage);
    let state_value = second_module.get_state_value();
    assert_eq!(state_value, Some(2));
}
