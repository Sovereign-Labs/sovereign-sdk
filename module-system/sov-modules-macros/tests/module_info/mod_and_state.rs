use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_api::{Context, Module, ModuleInfo, StateMap, WorkingSet};

pub mod first_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct FirstTestStruct<C>
    where
        C: Context,
    {
        #[address]
        pub address: C::Address,

        #[state]
        pub state_in_first_struct_1: StateMap<C::PublicKey, u32>,

        #[state]
        pub state_in_first_struct_2: StateMap<String, String>,
    }

    impl<C: Context> Module for FirstTestStruct<C> {
        type Context = C;

        type Config = ();

        type CallMessage = ();

        type Event = ();

        fn call(
            &self,
            _message: Self::CallMessage,
            _context: &Self::Context,
            _working_set: &mut WorkingSet<Self::Context>,
        ) -> Result<sov_modules_api::CallResponse, sov_modules_api::Error> {
            todo!()
        }
    }
}

mod second_test_module {
    use super::*;
    use sov_modules_api::Module;

    #[derive(ModuleInfo)]
    pub(crate) struct SecondTestStruct<C: Context> {
        #[address]
        pub address: C::Address,

        #[state]
        pub state_in_second_struct_1: StateMap<String, u32>,

        #[module]
        pub module_in_second_struct_1: first_test_module::FirstTestStruct<C>,
    }

    impl<C: Context> Module for SecondTestStruct<C> {
        type Context = C;

        type Config = ();

        type CallMessage = ();

        type Event = ();

        fn call(
            &self,
            _message: Self::CallMessage,
            _context: &Self::Context,
            _working_set: &mut WorkingSet<Self::Context>,
        ) -> Result<sov_modules_api::CallResponse, sov_modules_api::Error> {
            todo!()
        }
    }
}

fn main() {
    type C = ZkDefaultContext;
    let second_test_struct =
        <second_test_module::SecondTestStruct<C> as std::default::Default>::default();

    let prefix2 = second_test_struct.state_in_second_struct_1.prefix();
    assert_eq!(
        *prefix2,
        sov_modules_api::ModulePrefix::new_storage(
            // The tests compile inside trybuild.
            "trybuild001::second_test_module",
            "SecondTestStruct",
            "state_in_second_struct_1",
        )
        .into()
    );

    let prefix1 = second_test_struct
        .module_in_second_struct_1
        .state_in_first_struct_1
        .prefix();

    assert_eq!(
        *prefix1,
        sov_modules_api::ModulePrefix::new_storage(
            // The tests compile inside trybuild.
            "trybuild001::first_test_module",
            "FirstTestStruct",
            "state_in_first_struct_1"
        )
        .into()
    );

    assert_eq!(
        second_test_struct.dependencies(),
        [second_test_struct.module_in_second_struct_1.address()]
    );
}
