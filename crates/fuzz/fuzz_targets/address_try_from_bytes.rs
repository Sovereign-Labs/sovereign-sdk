#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_modules_api::Address;

fuzz_target!(|data: &[u8]| {
    let _ = Address::try_from(data);
});
