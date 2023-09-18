use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_api::{Context, ModuleInfo, StateMap, StateValue};

mod test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub(crate) struct TestStruct<C: Context> {
        #[address]
        pub address: C::Address,

        // Comment
        #[state]
        pub test_state1: StateMap<C::PublicKey, u32>,

        /// Doc comment
        #[state]
        pub test_state2: StateMap<String, String>,

        #[state]
        pub test_state3: StateValue<String>,
    }
}

fn main() {
    type C = ZkDefaultContext;
    let test_struct = <test_module::TestStruct<C> as std::default::Default>::default();

    let prefix1 = test_struct.test_state1.prefix();

    assert_eq!(
        *prefix1,
        sov_modules_api::Prefix::new_storage(
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
        sov_modules_api::Prefix::new_storage(
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
        sov_modules_api::Prefix::new_storage(
            // The tests compile inside trybuild.
            "trybuild000::test_module",
            "TestStruct",
            "test_state3"
        )
        .into()
    );

    use sov_modules_api::digest::Digest;
    let mut hasher = <C as sov_modules_api::Spec>::Hasher::new();
    hasher.update("trybuild000::test_module/TestStruct/".as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();

    assert_eq!(
        &sov_modules_api::Address::try_from(hash).unwrap(),
        test_struct.address()
    );
}
