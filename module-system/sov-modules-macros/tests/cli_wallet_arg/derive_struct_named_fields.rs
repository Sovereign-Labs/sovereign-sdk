use clap::Parser;
use sov_modules_macros::CliWalletArg;

#[derive(CliWalletArg, Debug, PartialEq)]
pub struct MyStruct {
    first_field: u32, str_field: String ,
}

fn main() {
    let expected = MyStruct{ first_field: 1, str_field: "hello".to_string()} ;
    let actual =
        <MyStruct as sov_modules_api::CliWalletArg>::CliStringRepr::try_parse_from(&["mystruct", "1", "hello"])
            .expect("parsing must succed")
            .into();
    assert_eq!(expected, actual);
}
