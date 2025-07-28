#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_modules_api::Address;

fuzz_target!(|data: &[u8]| {
    serde_json::from_slice::<Address>(data).ok();
});
