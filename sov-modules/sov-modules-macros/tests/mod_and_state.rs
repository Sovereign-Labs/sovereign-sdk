pub mod utils;

use sov_modules_macros::ModuleInfo;
use utils::{Context, TestContext, TestState, TestStorage};

pub mod first_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct FirstTestStruct<C: Context> {
        #[state]
        pub state_in_first_struct_1: TestState<C::Storage>,

        #[state]
        pub state_in_first_struct_2: TestState<C::Storage>,
    }
}

mod second_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct SecondTestStruct<C: Context> {
        #[state]
        pub state_in_second_struct_1: TestState<C::Storage>,

        #[module]
        pub module_in_second_struct_1: first_test_module::FirstTestStruct<C>,
    }
}

fn main() {
    let test_storage = TestStorage {};
    let second_test_struct =
        second_test_module::SecondTestStruct::<TestContext>::_new(test_storage);

    let prefix2 = second_test_struct.state_in_second_struct_1.prefix;
    assert_eq!(
        prefix2,
        sov_modules_api::Prefix::new(
            // The tests compile inside trybuild.
            "trybuild001::second_test_module".to_owned(),
            "SecondTestStruct".to_owned(),
            "state_in_second_struct_1".to_owned()
        )
    );

    let prefix1 = second_test_struct
        .module_in_second_struct_1
        .state_in_first_struct_1
        .prefix;

    assert_eq!(
        prefix1,
        sov_modules_api::Prefix::new(
            // The tests compile inside trybuild.
            "trybuild001::first_test_module".to_owned(),
            "FirstTestStruct".to_owned(),
            "state_in_first_struct_1".to_owned()
        )
    );
}
