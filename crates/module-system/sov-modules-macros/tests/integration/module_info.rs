#![allow(unused)]

use sov_modules_api::{
    Context, CryptoSpec, InnerEnumVariant, Module, ModuleId, ModuleInfo, Spec, StateMap,
    StateValue, TxState,
};
use sov_test_utils::ZkTestSpec;

mod test_module {
    use sov_modules_api::DaSpec;

    use super::*;

    #[derive(Clone, ModuleInfo)]
    #[module_info(sequencer_safety = "not_sequencer_safe")]
    pub(crate) struct TestStruct<S: Spec> {
        #[id]
        pub id: ModuleId,

        // Comment
        #[state]
        pub test_state1: StateMap<S::Address, u32>,

        /// Doc comment
        #[state]
        pub test_state2: StateMap<String, String>,

        #[state]
        pub test_state3: StateValue<String>,
    }

    fn not_sequencer_safe<S: Spec>(
        _module: &TestStruct<S>,
        _call: InnerEnumVariant<'_>,
        _address: &<<S as Spec>::Da as DaSpec>::Address,
    ) -> bool {
        false
    }

    impl<S: Spec> Module for TestStruct<S> {
        type Spec = S;
        type Config = ();
        type CallMessage = ();
        type Event = ();

        fn call(
            &mut self,
            _message: Self::CallMessage,
            _context: &Context<Self::Spec>,
            _state: &mut impl TxState<S>,
        ) -> anyhow::Result<()> {
            todo!()
        }
    }
}

#[test]
fn state_prefix_calculation() {
    let test_struct = test_module::TestStruct::<ZkTestSpec>::default();
    let prefix1 = test_struct.test_state1.prefix();

    assert_eq!(
        *prefix1,
        sov_modules_api::ModulePrefix::new_storage(
            "tests::module_info::test_module",
            "TestStruct",
            "test_state1"
        )
        .into()
    );

    let prefix2 = test_struct.test_state2.prefix();
    assert_eq!(
        *prefix2,
        sov_modules_api::ModulePrefix::new_storage(
            "tests::module_info::test_module",
            "TestStruct",
            "test_state2"
        )
        .into()
    );

    let prefix2 = test_struct.test_state3.prefix();
    assert_eq!(
        *prefix2,
        sov_modules_api::ModulePrefix::new_storage(
            "tests::module_info::test_module",
            "TestStruct",
            "test_state3"
        )
        .into()
    );
}

#[test]
fn module_id() {
    use sov_modules_api::digest::Digest;

    let test_struct = test_module::TestStruct::<ZkTestSpec>::default();

    let mut hasher = <<ZkTestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher::new();
    hasher.update("tests::module_info::test_module/TestStruct/".as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();

    assert_eq!(&sov_modules_api::ModuleId::from(hash), test_struct.id());
}

mod second_test_module {
    use super::*;

    #[derive(Clone, ModuleInfo)]
    pub(crate) struct SecondTestStruct<S: Spec> {
        #[id]
        pub id: ModuleId,

        #[state]
        pub state_in_second_struct_1: StateMap<String, u32>,

        #[module]
        pub module_in_second_struct_1: test_module::TestStruct<S>,
    }

    impl<S: Spec> Module for SecondTestStruct<S> {
        type Spec = S;
        type Config = ();
        type CallMessage = ();
        type Event = ();

        fn call(
            &mut self,
            _message: Self::CallMessage,
            _context: &Context<Self::Spec>,
            _state: &mut impl TxState<S>,
        ) -> anyhow::Result<()> {
            todo!()
        }
    }
}

#[test]
fn state_prefix_calculation_of_module_with_child_module() {
    let second_test_struct: second_test_module::SecondTestStruct<ZkTestSpec> = Default::default();

    let prefix2 = second_test_struct.state_in_second_struct_1.prefix();
    assert_eq!(
        *prefix2,
        sov_modules_api::ModulePrefix::new_storage(
            "tests::module_info::second_test_module",
            "SecondTestStruct",
            "state_in_second_struct_1",
        )
        .into()
    );

    let prefix1 = second_test_struct
        .module_in_second_struct_1
        .test_state1
        .prefix();

    assert_eq!(
        *prefix1,
        sov_modules_api::ModulePrefix::new_storage(
            "tests::module_info::test_module",
            "TestStruct",
            "test_state1"
        )
        .into()
    );
}

#[test]
fn dependencies_of_module_with_child_module() {
    let second_test_struct: second_test_module::SecondTestStruct<ZkTestSpec> = Default::default();

    assert_eq!(
        second_test_struct.dependencies(),
        [second_test_struct.module_in_second_struct_1.id()]
    );
}

#[test]
fn sequencer_safety_of_modules() {
    let test_struct = test_module::TestStruct::<ZkTestSpec>::default();
    let second_test_struct: second_test_module::SecondTestStruct<ZkTestSpec> = Default::default();
    let call_1 = InnerEnumVariant::new_for_test(&());
    let call_2 = InnerEnumVariant::new_for_test(&());

    assert!(
        !test_struct.is_safe_for_sequencer(call_1, &[0u8; 32].into()),
        "test struct overrode its sequencer safety"
    );

    assert!(
        second_test_struct.is_safe_for_sequencer(call_2, &[0u8; 32].into()),
        "second test struct uses default sequencer safety"
    );
}
