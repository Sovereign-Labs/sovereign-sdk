use sov_modules_api::mocks::MockContext;
use sov_modules_api::{Context, ModuleInfo};
use sov_modules_macros::ModuleInfo;
use sov_state::{ProverStorage, StateMap};

pub mod first_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct FirstTestStruct<C>
    where
        C: Context,
    {
        #[state]
        pub state_in_first_struct_1: StateMap<C::PublicKey, u32, C::Storage>,

        #[state]
        pub state_in_first_struct_2: StateMap<String, String, C::Storage>,
    }
}

mod second_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct SecondTestStruct<C: Context> {
        #[state]
        pub state_in_second_struct_1: StateMap<String, u32, C::Storage>,

        #[module]
        pub module_in_second_struct_1: first_test_module::FirstTestStruct<C>,
    }
}

fn main() {
    type C = MockContext;
    let test_storage = ProverStorage::temporary();
    let working_set = sov_state::WorkingSet::new(test_storage);

    let second_test_struct =
        <second_test_module::SecondTestStruct<C> as ModuleInfo>::new(working_set);

    let prefix2 = second_test_struct.state_in_second_struct_1.prefix();
    assert_eq!(
        *prefix2,
        sov_modules_api::Prefix::new_storage(
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
        sov_modules_api::Prefix::new_storage(
            // The tests compile inside trybuild.
            "trybuild001::first_test_module",
            "FirstTestStruct",
            "state_in_first_struct_1"
        )
        .into()
    );
}
