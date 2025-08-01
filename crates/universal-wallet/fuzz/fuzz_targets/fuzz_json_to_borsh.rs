#![no_main]
// A function inside the `fuzz_target` macro raises this warning
#![allow(non_snake_case)]

use libfuzzer_sys::fuzz_target;
use sov_universal_wallet::schema::Schema;
use universal_wallet_fuzz::FuzzInput;

fuzz_target!(|input: FuzzInput| {
    let schema = Schema::of_single_type::<FuzzInput>().unwrap();
    let data = serde_json::to_string(&input).unwrap();
    let json_serialized = schema.json_to_borsh(0, &data).unwrap();
    let borsh = borsh::to_vec(&input).unwrap();

    assert_eq!(json_serialized, borsh);
});
