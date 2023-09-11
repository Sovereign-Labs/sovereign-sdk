#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_celestia_adapter::shares::Share;

fuzz_target!(|data: &[u8]| {
    serde_json::from_slice::<Share>(data).ok();
});
