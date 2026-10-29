#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_accounts::CallMessage;

type S = sov_test_utils::TestSpec;

fuzz_target!(|input: CallMessage<S>| {
    let json = serde_json::to_vec(&input).unwrap();
    let msg = serde_json::from_slice::<CallMessage<S>>(&json).unwrap();
    assert_eq!(input, msg);
});
