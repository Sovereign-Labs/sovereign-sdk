#![no_main]
// A function inside the `fuzz_target` macro raises this warning
#![allow(non_snake_case)]

use libfuzzer_sys::fuzz_target;
use sov_universal_wallet::schema::Schema;
use universal_wallet_fuzz::FuzzInput;

fuzz_target!(|input: FuzzInput| {
    let schema = Schema::of_single_type::<FuzzInput>().unwrap();
    let bytes = borsh::to_vec(&input).unwrap();

    assert!(schema.display(0, &bytes).is_ok());
});
