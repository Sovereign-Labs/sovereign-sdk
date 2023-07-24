use clap::Parser;
use sov_modules_api::macros::CliWalletArg;

#[derive(CliWalletArg, Debug, PartialEq)]
pub enum MyEnum {
    Foo(u32, String),
    Bar(u8),
}

fn main() {
    let expected_foo = MyEnum::Foo(1, "hello".to_string());
    let actual_foo = <MyEnum as sov_modules_api::CliWalletArg>::CliStringRepr::try_parse_from(&[
        "myenum", "foo", "1", "hello",
    ])
    .expect("parsing must succed")
    .into();
    assert_eq!(expected_foo, actual_foo);

    let expected_bar = MyEnum::Bar(2);
    let actual_bar = <MyEnum as sov_modules_api::CliWalletArg>::CliStringRepr::try_parse_from(&[
        "myenum", "bar", "2",
    ])
    .expect("parsing must succed")
    .into();

    assert_eq!(expected_bar, actual_bar);
}
