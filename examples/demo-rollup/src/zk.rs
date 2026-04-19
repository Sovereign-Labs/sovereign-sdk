//! Small utilities for zk tooling

use std::sync::Arc;

/// Returns the SP1 host arguments for a rollup with mock da. This is the code that is zk-proven by the rollup
pub fn mock_da_sp1_host_args() -> Arc<&'static [u8]> {
    // Don't try to read the elf file if we're not building the sp1 guest!
    if sov_zkvm_utils::should_skip_guest_build("sp1") {
        return Arc::new(vec![].leak());
    }

    Arc::new(*sp1::SP1_GUEST_MOCK_ELF)
}

/// Returns the host arguments for a rollup with mock zkvm as inner VM.
/// `MockZkvmHost::HostArgs` is `()`, so there's nothing to pass.
pub fn mock_zkvm_host_args() -> Arc<()> {
    Arc::new(())
}

/// Returns the risc0 host arguments for a rollup with mock da. This is the code that is zk-proven by the rollup
pub fn mock_da_risc0_host_args() -> Arc<&'static [u8]> {
    // Don't try to read the elf file if we're not building the risc0 guest!
    if sov_zkvm_utils::should_skip_guest_build("risc0") {
        return Arc::new(vec![].leak());
    }

    Arc::new(risc0::MOCK_DA_ELF)
}

/// Returns the risc0 host arguments for a rollup with celestia da. This is the code that is zk-proven by the rollup
pub fn celestia_risc0_host_args() -> Arc<&'static [u8]> {
    // Don't try to read the elf file if we're not building the risc0 guest!
    if sov_zkvm_utils::should_skip_guest_build("risc0") {
        return Arc::new(vec![].leak());
    }

    Arc::new(risc0::ROLLUP_ELF)
}
