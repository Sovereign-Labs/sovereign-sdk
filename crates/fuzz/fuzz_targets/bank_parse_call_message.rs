#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_bank::CallMessage;

type S = sov_test_utils::TestSpec;

fuzz_target!(|input: &[u8]| {
    serde_json::from_slice::<CallMessage<S>>(input).ok();
});
