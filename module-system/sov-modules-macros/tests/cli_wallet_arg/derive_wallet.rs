use clap::Parser;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::macros::{CliWallet, CliWalletArg, DefaultRuntime};
use sov_modules_api::{
    CallResponse, Context, DispatchCall, Error, Genesis, MessageCodec, Module, ModuleInfo,
};
use sov_state::{StateValue, WorkingSet};

pub mod first_test_module {
    use super::*;

    #[derive(CliWalletArg, Debug, PartialEq, borsh::BorshDeserialize, borsh::BorshSerialize)]
    pub struct MyStruct {
        pub first_field: u32,
        pub str_field: String,
    }

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
        type CallMessage = MyStruct;

        fn genesis(
            &self,
            _config: &Self::Config,
            _working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<(), Error> {
            Ok(())
        }

        fn call(
            &self,
            _msg: Self::CallMessage,
            _context: &Self::Context,
            _working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<CallResponse, Error> {
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

    #[derive(CliWalletArg, Debug, PartialEq, borsh::BorshDeserialize, borsh::BorshSerialize)]
    pub enum MyEnum {
        Foo { first_field: u32, str_field: String },
        Bar(u8),
    }

    impl<Ctx: Context> Module for SecondTestStruct<Ctx> {
        type Context = Ctx;
        type Config = ();
        type CallMessage = MyEnum;

        fn genesis(
            &self,
            _config: &Self::Config,
            _working_set: &mut WorkingSet<Ctx::Storage>,
        ) -> Result<(), Error> {
            Ok(())
        }

        fn call(
            &self,
            _msg: Self::CallMessage,
            _context: &Self::Context,
            _working_set: &mut WorkingSet<Ctx::Storage>,
        ) -> Result<CallResponse, Error> {
            Ok(CallResponse::default())
        }
    }
}

#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime, CliWallet)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Runtime<C: Context> {
    pub first: first_test_module::FirstTestStruct<C>,
    pub second: second_test_module::SecondTestStruct<C>,
}

fn main() {
    let expected_foo = RuntimeCall::first(first_test_module::MyStruct {
        first_field: 1,
        str_field: "hello".to_string(),
    });
    let actual_foo =
        <Runtime<DefaultContext> as sov_modules_api::CliWallet>::CliStringRepr::try_parse_from(&[
            "main",
            "first",
            "my-struct",
            "1",
            "hello",
        ])
        .expect("parsing must succed")
        .into();
    assert_eq!(expected_foo, actual_foo);

    let expected_bar = RuntimeCall::second(second_test_module::MyEnum::Bar(2));
    let actual_bar =
        <Runtime<DefaultContext> as sov_modules_api::CliWallet>::CliStringRepr::try_parse_from(&[
            "main", "second", "bar", "2",
        ])
        .expect("parsing must succed")
        .into();

    assert_eq!(expected_bar, actual_bar);
}
