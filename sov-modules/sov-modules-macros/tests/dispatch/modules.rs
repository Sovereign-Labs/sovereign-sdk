use sov_modules_api::{CallResponse, Context, Error, Module};
use sov_modules_macros::ModuleInfo;
use sov_state::StateValue;

pub mod first_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct FirstTestStruct<C: Context> {
        #[state]
        pub state_in_first_struct: StateValue<u8, C::Storage>,
    }

    impl<C: Context> Module for FirstTestStruct<C> {
        type Context = C;
        type CallMessage = u8;
        type QueryMessage = ();

        fn genesis(&mut self) -> Result<(), Error> {
            self.state_in_first_struct.set(1);
            Ok(())
        }

        fn call(
            &mut self,
            msg: Self::CallMessage,
            _context: &Self::Context,
        ) -> Result<CallResponse, Error> {
            self.state_in_first_struct.set(msg);
            Ok(CallResponse::default())
        }

        fn query(&self, _msg: Self::QueryMessage) -> sov_modules_api::QueryResponse {
            let state = self.state_in_first_struct.get().unwrap();
            sov_modules_api::QueryResponse {
                response: vec![state],
            }
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
        pub state_in_second_struct: StateValue<u8, C::Storage>,
    }

    impl<C: Context> Module for SecondTestStruct<C> {
        type Context = C;
        type CallMessage = u8;
        type QueryMessage = TestType;

        fn genesis(&mut self) -> Result<(), Error> {
            self.state_in_second_struct.set(2);
            Ok(())
        }

        fn call(
            &mut self,
            msg: Self::CallMessage,
            _context: &Self::Context,
        ) -> Result<CallResponse, Error> {
            self.state_in_second_struct.set(msg);
            Ok(CallResponse::default())
        }

        fn query(&self, _msg: Self::QueryMessage) -> sov_modules_api::QueryResponse {
            let state = self.state_in_second_struct.get().unwrap();
            sov_modules_api::QueryResponse {
                response: vec![state],
            }
        }
    }
}
