#![no_main]
#[macro_use]
extern crate libfuzzer_sys;

use std::str::FromStr;

use sov_modules_api::AddressBech32;

fuzz_target!(|data: &[u8]| {
    if let Ok(data) = std::str::from_utf8(data) {
        if let Ok(addr) = AddressBech32::from_str(data) {
            addr.to_string();
        }
    }
});
