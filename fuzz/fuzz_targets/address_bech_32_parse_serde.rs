#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_modules_api::AddressBech32;

fuzz_target!(|data: &[u8]| {
    serde_json::from_slice::<AddressBech32>(data).ok();
});
