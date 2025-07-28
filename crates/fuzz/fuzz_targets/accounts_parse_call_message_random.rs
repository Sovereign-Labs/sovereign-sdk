#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_accounts::CallMessage;

fuzz_target!(|input: &[u8]| {
    serde_json::from_slice::<CallMessage>(input).ok();
});
