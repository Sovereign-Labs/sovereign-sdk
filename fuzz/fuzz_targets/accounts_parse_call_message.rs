#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_accounts::CallMessage;
use sov_modules_api::default_context::DefaultContext;

type C = DefaultContext;

fuzz_target!(|input: CallMessage<C>| {
    let json = serde_json::to_vec(&input).unwrap();
    let msg = serde_json::from_slice::<CallMessage<C>>(&json).unwrap();
    assert_eq!(input, msg);
});
