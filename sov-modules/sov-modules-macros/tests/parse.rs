use sov_modules_api::mocks::MockContext;
use sov_modules_api::{Context, ModuleInfo};
use sov_modules_macros::ModuleInfo;
use sov_state::{JmtStorage, StateMap, StateValue};

mod test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct TestStruct<C: Context> {
        // Comment
        #[state]
        pub test_state1: StateMap<C::PublicKey, u32, C::Storage>,

        /// Doc comment
        #[state]
        pub test_state2: StateMap<String, String, C::Storage>,

        #[state]
        pub test_state3: StateValue<String, C::Storage>,
    }
}

fn main() {
    type C = MockContext;
    let test_storage = JmtStorage::default();
    let test_struct = <test_module::TestStruct<C> as ModuleInfo<C>>::new(test_storage);

    let prefix1 = test_struct.test_state1.prefix();

    assert_eq!(
        *prefix1,
        sov_modules_api::Prefix::new(
            // The tests compile inside trybuild.
            "trybuild000::test_module",
            "TestStruct",
            "test_state1"
        )
        .into()
    );

    let prefix2 = test_struct.test_state2.prefix();
    assert_eq!(
        *prefix2,
        sov_modules_api::Prefix::new(
            // The tests compile inside trybuild.
            "trybuild000::test_module",
            "TestStruct",
            "test_state2"
        )
        .into()
    );

    let prefix2 = test_struct.test_state3.prefix();
    assert_eq!(
        *prefix2,
        sov_modules_api::Prefix::new(
            // The tests compile inside trybuild.
            "trybuild000::test_module",
            "TestStruct",
            "test_state3"
        )
        .into()
    );
}
