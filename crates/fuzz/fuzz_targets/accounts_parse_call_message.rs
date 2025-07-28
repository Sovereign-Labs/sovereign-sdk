#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_accounts::CallMessage;

fuzz_target!(|input: CallMessage| {
    let json = serde_json::to_vec(&input).unwrap();
    let msg = serde_json::from_slice::<CallMessage>(&json).unwrap();
    assert_eq!(input, msg);
});
