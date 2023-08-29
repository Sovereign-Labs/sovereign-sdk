#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_modules_api::AddressBech32;

fuzz_target!(|data: &[u8]| {
    let _ = AddressBech32::try_from(data);
});
