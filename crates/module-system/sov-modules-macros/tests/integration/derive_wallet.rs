use sov_modules_api::cli::JsonStringArg;
use sov_modules_api::macros::{CliWallet, UniversalWallet};
use sov_modules_api::{
    Context, DaSpec, DispatchCall, Genesis, MessageCodec, Module, ModuleId, ModuleInfo,
    SizedSafeString, Spec, StateValue, TxState,
};
use sov_test_utils::TestSpec;

pub mod first_test_module {
    use super::*;

    #[derive(
        Debug,
        PartialEq,
        Eq,
        Clone,
        borsh::BorshDeserialize,
        borsh::BorshSerialize,
        serde::Serialize,
        serde::Deserialize,
        schemars::JsonSchema,
        UniversalWallet,
    )]
    pub struct MyStruct {
        pub first_field: u32,
        pub str_field: SizedSafeString<10>,
    }

    #[derive(Clone, ModuleInfo, PartialEq, Eq)]
    pub struct FirstTestStruct<S: Spec> {
        #[id]
        pub id: ModuleId,

        #[state]
        pub state_in_first_struct: StateValue<u8>,

        #[phantom]
        phantom: std::marker::PhantomData<S>,
    }

    impl<S: Spec> Module for FirstTestStruct<S> {
        type Spec = S;
        type Config = ();
        type CallMessage = MyStruct;
        type Event = ();

        fn genesis(
            &mut self,
            _genesis_rollup_header: &<S::Da as DaSpec>::BlockHeader,

            _config: &Self::Config,
            _state: &mut impl sov_modules_api::GenesisState<S>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn call(
            &mut self,
            _msg: Self::CallMessage,
            _context: &Context<Self::Spec>,
            _state: &mut impl TxState<S>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }
}

pub mod second_test_module {
    use super::*;

    #[derive(Clone, ModuleInfo, PartialEq, Eq)]
    pub struct SecondTestStruct<S: Spec> {
        #[id]
        pub id: ModuleId,

        #[state]
        pub state_in_second_struct: StateValue<u8>,

        #[phantom]
        phantom: std::marker::PhantomData<S>,
    }

    #[derive(
        Debug,
        PartialEq,
        Eq,
        Clone,
        borsh::BorshDeserialize,
        borsh::BorshSerialize,
        serde::Serialize,
        serde::Deserialize,
        schemars::JsonSchema,
        UniversalWallet,
    )]
    pub enum MyEnum {
        Foo {
            first_field: u32,
            str_field: SizedSafeString<10>,
        },
        Bar(u8),
    }

    impl<S: Spec> Module for SecondTestStruct<S> {
        type Spec = S;
        type Config = ();
        type CallMessage = MyEnum;
        type Event = ();

        fn genesis(
            &mut self,
            _genesis_rollup_header: &<S::Da as DaSpec>::BlockHeader,

            _config: &Self::Config,
            _state: &mut impl sov_modules_api::GenesisState<S>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn call(
            &mut self,
            _msg: Self::CallMessage,
            _context: &Context<Self::Spec>,
            _state: &mut impl TxState<S>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }
}

#[derive(Default, Genesis, PartialEq, Eq, DispatchCall, MessageCodec, CliWallet)]
pub struct Runtime<S: Spec> {
    pub first: first_test_module::FirstTestStruct<S>,
    pub second: second_test_module::SecondTestStruct<S>,
}

#[test]
fn build_runtime_call() {
    use sov_modules_api::prelude::clap::Parser;

    let expected_foo = RuntimeCall::First(first_test_module::MyStruct {
        first_field: 1,
        str_field: SizedSafeString::<10>::try_from("hello").unwrap(),
    });
    let foo_from_cli: RuntimeSubcommand<JsonStringArg, TestSpec> =
        <RuntimeSubcommand<JsonStringArg, TestSpec>>::try_parse_from([
            "main",
            "first",
            "--json",
            r#"{"first_field": 1, "str_field": "hello"}"#,
            "--chain-id",
            "0",
            "--max-fee",
            "0",
        ])
        .expect("parsing must succeed");
    let foo_ir: RuntimeMessage<JsonStringArg, TestSpec> = foo_from_cli.try_into().unwrap();
    assert_eq!(expected_foo, foo_ir.try_into().unwrap());

    let expected_bar = RuntimeCall::Second(second_test_module::MyEnum::Bar(2));
    let bar_from_cli: RuntimeSubcommand<JsonStringArg, TestSpec> =
        <RuntimeSubcommand<JsonStringArg, TestSpec>>::try_parse_from([
            "main",
            "second",
            "--json",
            r#"{"Bar": 2}"#,
            "--chain-id",
            "0",
            "--max-fee",
            "0",
        ])
        .expect("parsing must succeed");
    let bar_ir: RuntimeMessage<JsonStringArg, TestSpec> = bar_from_cli.try_into().unwrap();

    assert_eq!(expected_bar, bar_ir.try_into().unwrap());
}
