use sov_modules_api::{CallResponse, Context, Error, Module};
use sov_modules_macros::ModuleInfo;
use sov_state::{StateValue, WorkingSet};

pub mod first_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub struct FirstTestStruct<C: Context> {
        #[address]
        pub address: C::Address,

        #[state]
        pub state_in_first_struct: StateValue<u8>,
    }

    impl<C: Context> Module for FirstTestStruct<C> {
        type Context = C;
        type Config = ();
        type CallMessage = u8;
        type QueryMessage = ();

        fn genesis(
            &self,
            _config: &Self::Config,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<(), Error> {
            self.state_in_first_struct.set(1, working_set);
            Ok(())
        }

        fn call(
            &self,
            msg: Self::CallMessage,
            _context: &Self::Context,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<CallResponse, Error> {
            self.state_in_first_struct.set(msg, working_set);
            Ok(CallResponse::default())
        }

        fn query(
            &self,
            _msg: Self::QueryMessage,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> sov_modules_api::QueryResponse {
            let state = self.state_in_first_struct.get(working_set).unwrap();
            sov_modules_api::QueryResponse {
                response: vec![state],
            }
        }
    }
}

pub mod second_test_module {
    use super::*;

    #[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Clone, PartialEq, Debug)]
    pub struct TestType {}

    #[derive(ModuleInfo)]
    pub struct SecondTestStruct<C: Context> {
        #[address]
        pub address: C::Address,

        #[state]
        pub state_in_second_struct: StateValue<u8>,
    }

    impl<C: Context> Module for SecondTestStruct<C> {
        type Context = C;
        type Config = ();
        type CallMessage = u8;
        type QueryMessage = TestType;

        fn genesis(
            &self,
            _config: &Self::Config,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<(), Error> {
            self.state_in_second_struct.set(2, working_set);
            Ok(())
        }

        fn call(
            &self,
            msg: Self::CallMessage,
            _context: &Self::Context,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<CallResponse, Error> {
            self.state_in_second_struct.set(msg, working_set);
            Ok(CallResponse::default())
        }

        fn query(
            &self,
            _msg: Self::QueryMessage,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> sov_modules_api::QueryResponse {
            let state = self.state_in_second_struct.get(working_set).unwrap();
            sov_modules_api::QueryResponse {
                response: vec![state],
            }
        }
    }
}
