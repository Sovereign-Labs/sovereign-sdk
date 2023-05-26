use sov_modules_api::{CallResponse, Context, Error, Genesis, Module};
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

    impl<C: Context> FirstTestStruct<C> {
        pub fn get_state_value(&self, working_set: &mut WorkingSet<C::Storage>) -> u8 {
            self.state_in_first_struct.get(working_set).unwrap()
        }
    }

    impl<C: Context> Genesis for FirstTestStruct<C> {
        type Context = C;
        type Config = ();

        fn genesis(
            &self,
            _config: &Self::Config,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<(), Error> {
            self.state_in_first_struct.set(1, working_set);
            Ok(())
        }
    }

    impl<C: Context> Module for FirstTestStruct<C> {
        type CallMessage = u8;

        fn call(
            &self,
            msg: Self::CallMessage,
            _context: &Self::Context,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<CallResponse, Error> {
            self.state_in_first_struct.set(msg, working_set);
            Ok(CallResponse::default())
        }
    }
}

pub mod second_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub struct SecondTestStruct<Ctx: Context> {
        #[address]
        pub address: Ctx::Address,

        #[state]
        pub state_in_second_struct: StateValue<u8>,
    }

    impl<Ctx: Context> SecondTestStruct<Ctx> {
        pub fn get_state_value(&self, working_set: &mut WorkingSet<Ctx::Storage>) -> u8 {
            self.state_in_second_struct.get(working_set).unwrap()
        }
    }

    impl<Ctx: Context> Genesis for SecondTestStruct<Ctx> {
        type Context = Ctx;
        type Config = ();

        fn genesis(
            &self,
            _config: &Self::Config,
            working_set: &mut WorkingSet<Ctx::Storage>,
        ) -> Result<(), Error> {
            self.state_in_second_struct.set(2, working_set);
            Ok(())
        }
    }

    impl<Ctx: Context> Module for SecondTestStruct<Ctx> {
        type CallMessage = u8;

        fn call(
            &self,
            msg: Self::CallMessage,
            _context: &Self::Context,
            working_set: &mut WorkingSet<Ctx::Storage>,
        ) -> Result<CallResponse, Error> {
            self.state_in_second_struct.set(msg, working_set);
            Ok(CallResponse::default())
        }
    }
}
