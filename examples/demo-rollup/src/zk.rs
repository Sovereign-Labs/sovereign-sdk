//! Small utilities for zk tooling

use std::sync::Arc;

/// Returns the risc0 host arguments for a rollup with mock da. This is the code that is zk-proven by the rollup
pub fn mock_da_risc0_host_args() -> Arc<&'static [u8]> {
    // Don't try to read the elf file if we're not building the risc0 guest!
    if should_skip_guest_build() {
        return Arc::new(vec![].leak());
    }

    Arc::new(risc0::MOCK_DA_ELF)
}

/// Returns the risc0 host arguments for a rollup with celestia da. This is the code that is zk-proven by the rollup
pub fn celestia_risc0_host_args() -> Arc<&'static [u8]> {
    // Don't try to read the elf file if we're not building the risc0 guest!
    if should_skip_guest_build() {
        return Arc::new(vec![].leak());
    }

    Arc::new(risc0::ROLLUP_ELF)
}

fn should_skip_guest_build() -> bool {
    match std::env::var("SKIP_GUEST_BUILD")
        .as_ref()
        .map(|arg0: &String| String::as_str(arg0))
    {
        Ok("1") | Ok("true") | Ok("risc0") => true,
        Ok("0") | Ok("false") | Ok(_) | Err(_) => false,
    }
}
