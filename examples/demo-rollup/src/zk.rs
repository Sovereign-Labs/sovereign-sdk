//! Feature-gated inner ZKVM selection.
//!
//! ZKVM priority: `mock_zkvm` > `risc0` > `sp1`.
//! When multiple ZKVM features are enabled (e.g. `--all-features`), `mock_zkvm` wins.

// Outer VM is always MockZkvm regardless of inner VM selection
pub use sov_mock_zkvm::MockCodeCommitment;
pub use sov_mock_zkvm::MockZkvm as OuterZkvm;
pub use sov_mock_zkvm::MockZkvmHost as OuterZkvmHost;

// ---------------------------------------------------------------------------
// Inner ZKVM: mock (highest priority, and ultimate fallback if nothing is set)
// ---------------------------------------------------------------------------
#[cfg(any(
    feature = "mock_zkvm",
    all(not(feature = "risc0"), not(feature = "sp1"))
))]
mod inner {
    use std::sync::Arc;

    use sov_mock_zkvm::{MockZkvm, MockZkvmCryptoSpec, MockZkvmHost};
    use sov_rollup_interface::zk::CryptoSpec;

    #[allow(missing_docs)]
    pub type InnerZkvm = MockZkvm;
    #[allow(missing_docs)]
    pub type InnerCryptoSpec = MockZkvmCryptoSpec;
    #[allow(missing_docs)]
    pub type Hasher = <MockZkvmCryptoSpec as CryptoSpec>::Hasher;

    /// Returns host arguments for the mock inner ZKVM (unit type).
    pub fn mock_da_host_args() -> Arc<()> {
        Arc::new(())
    }

    /// Returns host arguments for celestia DA with mock inner ZKVM (unit type).
    #[cfg(feature = "celestia_da")]
    pub fn celestia_host_args() -> Arc<()> {
        Arc::new(())
    }

    /// Creates the inner VM from a prover config, returning the VM and the config discriminant.
    pub fn create_inner_vm(
        prover_config: sov_stf_runner::processes::RollupProverConfig<InnerZkvm>,
    ) -> (
        MockZkvmHost,
        sov_stf_runner::processes::RollupProverConfigDiscriminants,
    ) {
        let (_, disc) = prover_config.split();
        (MockZkvmHost::new_non_blocking(), disc)
    }
}

// ---------------------------------------------------------------------------
// Inner ZKVM: risc0 (only when mock_zkvm is NOT enabled)
// ---------------------------------------------------------------------------
#[cfg(all(feature = "risc0", not(feature = "mock_zkvm")))]
mod inner {
    use std::sync::Arc;

    use sov_risc0_adapter::host::Risc0Host;
    use sov_risc0_adapter::{Risc0, Risc0CryptoSpec};
    use sov_rollup_interface::zk::CryptoSpec;

    #[allow(missing_docs)]
    pub type InnerZkvm = Risc0;
    #[allow(missing_docs)]
    pub type InnerCryptoSpec = Risc0CryptoSpec;
    #[allow(missing_docs)]
    pub type Hasher = <Risc0CryptoSpec as CryptoSpec>::Hasher;

    /// Returns the risc0 host arguments for a rollup with mock DA.
    #[cfg(feature = "mock_da")]
    pub fn mock_da_host_args() -> Arc<&'static [u8]> {
        if sov_zkvm_utils::should_skip_guest_build("risc0") {
            return Arc::new(&[]);
        }
        Arc::new(risc0_prover::MOCK_DA_ELF)
    }

    /// Returns the risc0 host arguments for a rollup with celestia DA.
    #[cfg(feature = "celestia_da")]
    pub fn celestia_host_args() -> Arc<&'static [u8]> {
        if sov_zkvm_utils::should_skip_guest_build("risc0") {
            return Arc::new(&[]);
        }
        Arc::new(risc0_prover::ROLLUP_ELF)
    }

    /// Creates the inner VM from a prover config, returning the VM and the config discriminant.
    pub fn create_inner_vm(
        prover_config: sov_stf_runner::processes::RollupProverConfig<InnerZkvm>,
    ) -> (
        Risc0Host<'static>,
        sov_stf_runner::processes::RollupProverConfigDiscriminants,
    ) {
        let (host_args, disc) = prover_config.split();
        (Risc0Host::new(*host_args), disc)
    }
}

// ---------------------------------------------------------------------------
// Inner ZKVM: sp1 (only when mock_zkvm and risc0 are NOT enabled)
// ---------------------------------------------------------------------------
#[cfg(all(feature = "sp1", not(feature = "mock_zkvm"), not(feature = "risc0")))]
mod inner {
    use std::sync::Arc;

    use sov_rollup_interface::zk::CryptoSpec;
    use sov_sp1_adapter::host::SP1Host;
    use sov_sp1_adapter::{SP1CryptoSpec, SP1};

    #[allow(missing_docs)]
    pub type InnerZkvm = SP1;
    #[allow(missing_docs)]
    pub type InnerCryptoSpec = SP1CryptoSpec;
    #[allow(missing_docs)]
    pub type Hasher = <SP1CryptoSpec as CryptoSpec>::Hasher;

    /// Returns the sp1 host arguments for a rollup with mock DA.
    #[cfg(feature = "mock_da")]
    pub fn mock_da_host_args() -> Arc<&'static [u8]> {
        if sov_zkvm_utils::should_skip_guest_build("sp1") {
            return Arc::new(&[]);
        }
        Arc::new(&sp1_prover::SP1_GUEST_MOCK_ELF)
    }

    /// Returns the sp1 host arguments for a rollup with celestia DA.
    #[cfg(feature = "celestia_da")]
    pub fn celestia_host_args() -> Arc<&'static [u8]> {
        if sov_zkvm_utils::should_skip_guest_build("sp1") {
            return Arc::new(&[]);
        }
        Arc::new(&sp1_prover::SP1_GUEST_CELESTIA_ELF)
    }

    /// Creates the inner VM from a prover config, returning the VM and the config discriminant.
    pub fn create_inner_vm(
        prover_config: sov_stf_runner::processes::RollupProverConfig<InnerZkvm>,
    ) -> (
        SP1Host,
        sov_stf_runner::processes::RollupProverConfigDiscriminants,
    ) {
        let (host_args, disc) = prover_config.split();
        let host = SP1Host::new(*host_args).expect("SP1Host should be created successfully");
        (host, disc)
    }
}

// Re-export the active inner ZKVM module.
// Falls back to mock_zkvm if no ZKVM feature is explicitly set (see `mod inner` above).
pub use inner::*;
