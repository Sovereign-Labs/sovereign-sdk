pub mod utils;

use sov_modules_macros::ModuleInfo;

use utils::{Context, TestContext, TestState, TestStorage};

mod test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct TestStruct<C: Context> {
        #[state]
        pub test_state1: TestState<C::Storage>,

        #[state]
        pub test_state2: TestState<C::Storage>,
    }
}

fn main() {
    let test_storage = TestStorage {};
    let test_struct = test_module::TestStruct::<TestContext>::_new(test_storage);

    let prefix1 = test_struct.test_state1.prefix;
    assert_eq!(
        prefix1,
        sov_modules_api::Prefix::new(
            // The tests compile inside trybuild.
            "trybuild000::test_module".to_owned(),
            "TestStruct".to_owned(),
            "test_state1".to_owned()
        )
    );

    let prefix2 = test_struct.test_state2.prefix;
    assert_eq!(
        prefix2,
        sov_modules_api::Prefix::new(
            // The tests compile inside trybuild.
            "trybuild000::test_module".to_owned(),
            "TestStruct".to_owned(),
            "test_state2".to_owned()
        )
    );
}
