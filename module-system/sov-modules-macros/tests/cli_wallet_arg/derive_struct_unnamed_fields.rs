use clap::Parser;
use sov_modules_api::macros::CliWalletArg;

#[derive(CliWalletArg, Debug, PartialEq)]
pub struct MyStruct(u32, String);

fn main() {
    let expected = MyStruct(1, "hello".to_string());
    let actual = <MyStruct as sov_modules_api::CliWalletArg>::CliStringRepr::try_parse_from(&[
        "main",
        "my-struct",
        "1",
        "hello",
    ])
    .expect("parsing must succed")
    .into();
    assert_eq!(expected, actual);
}
