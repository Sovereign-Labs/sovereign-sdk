#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_universal_wallet::schema::Schema;
use universal_wallet_fuzz::FuzzInput;

fuzz_target!(|input: FuzzInput| {
    let schema = Schema::of_single_type::<FuzzInput>().unwrap();
    let bytes = borsh::to_vec(&input).unwrap();

    assert!(schema.display(0, &bytes).is_ok());
});
