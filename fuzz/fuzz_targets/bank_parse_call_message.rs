#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_bank::CallMessage;
use sov_modules_api::default_context::DefaultContext;

type C = DefaultContext;

fuzz_target!(|input: &[u8]| {
    serde_json::from_slice::<CallMessage<C>>(input).ok();
});
