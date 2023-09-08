#![no_main]
#[macro_use]
extern crate libfuzzer_sys;

use sov_celestia_adapter::shares::NamespaceGroup;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = NamespaceGroup::from_b64(s).ok();
    }
});
