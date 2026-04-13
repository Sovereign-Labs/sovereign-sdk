//! Feature-gated inner ZKVM selection.
//!
//! Exactly one of `mock_zkvm`, `risc0`, or `sp1` features must be enabled.
//! This module re-exports the selected inner ZKVM types under unified names.

// Outer VM is always MockZkvm regardless of inner VM selection
pub use sov_mock_zkvm::MockCodeCommitment;
pub use sov_mock_zkvm::MockZkvm as OuterZkvm;
pub use sov_mock_zkvm::MockZkvmHost as OuterZkvmHost;

/// Creates the outer VM host.
pub fn create_outer_vm() -> OuterZkvmHost {
    OuterZkvmHost::new_non_blocking()
}

// ---------------------------------------------------------------------------
// Inner ZKVM: mock
// ---------------------------------------------------------------------------
#[cfg(feature = "mock_zkvm")]
mod inner {
    use std::sync::Arc;

    use sov_mock_zkvm::{MockZkvm, MockZkvmCryptoSpec, MockZkvmHost};
    use sov_rollup_interface::zk::CryptoSpec;

    /// The inner ZKVM type used in `ConfigurableSpec`.
    pub type InnerZkvm = MockZkvm;

    /// The cryptographic specification matching the inner ZKVM.
    pub type InnerCryptoSpec = MockZkvmCryptoSpec;

    /// Hasher derived from the inner crypto spec.
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
// Inner ZKVM: risc0
// ---------------------------------------------------------------------------
#[cfg(feature = "risc0")]
mod inner {
    use std::sync::Arc;

    use sov_risc0_adapter::host::Risc0Host;
    use sov_risc0_adapter::{Risc0, Risc0CryptoSpec};
    use sov_rollup_interface::zk::CryptoSpec;

    /// The inner ZKVM type used in `ConfigurableSpec`.
    pub type InnerZkvm = Risc0;

    /// The cryptographic specification matching the inner ZKVM.
    pub type InnerCryptoSpec = Risc0CryptoSpec;

    /// Hasher derived from the inner crypto spec.
    pub type Hasher = <Risc0CryptoSpec as CryptoSpec>::Hasher;

    fn should_skip_guest_build() -> bool {
        match std::env::var("SKIP_GUEST_BUILD")
            .as_ref()
            .map(|arg0: &String| String::as_str(arg0))
        {
            Ok("1") | Ok("true") | Ok("risc0") => true,
            Ok("0") | Ok("false") | Ok(_) | Err(_) => false,
        }
    }

    /// Returns the risc0 host arguments for a rollup with mock DA.
    #[cfg(feature = "mock_da")]
    pub fn mock_da_host_args() -> Arc<&'static [u8]> {
        if should_skip_guest_build() {
            return Arc::new(vec![].leak());
        }
        Arc::new(risc0_prover::MOCK_DA_ELF)
    }

    /// Returns the risc0 host arguments for a rollup with celestia DA.
    #[cfg(feature = "celestia_da")]
    pub fn celestia_host_args() -> Arc<&'static [u8]> {
        if should_skip_guest_build() {
            return Arc::new(vec![].leak());
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
// Inner ZKVM: sp1
// ---------------------------------------------------------------------------
#[cfg(feature = "sp1")]
mod inner {
    use std::sync::Arc;

    use sov_rollup_interface::zk::CryptoSpec;
    use sov_sp1_adapter::host::SP1Host;
    use sov_sp1_adapter::{SP1CryptoSpec, SP1};

    /// The inner ZKVM type used in `ConfigurableSpec`.
    pub type InnerZkvm = SP1;

    /// The cryptographic specification matching the inner ZKVM.
    pub type InnerCryptoSpec = SP1CryptoSpec;

    /// Hasher derived from the inner crypto spec.
    pub type Hasher = <SP1CryptoSpec as CryptoSpec>::Hasher;

    fn should_skip_guest_build() -> bool {
        match std::env::var("SKIP_GUEST_BUILD")
            .as_ref()
            .map(|arg0: &String| String::as_str(arg0))
        {
            Ok("1") | Ok("true") | Ok("sp1") => true,
            Ok("0") | Ok("false") | Ok(_) | Err(_) => false,
        }
    }

    /// Returns the sp1 host arguments for a rollup with mock DA.
    #[cfg(feature = "mock_da")]
    pub fn mock_da_host_args() -> Arc<&'static [u8]> {
        if should_skip_guest_build() {
            return Arc::new(vec![].leak());
        }
        Arc::new(&sp1_prover::SP1_GUEST_MOCK_ELF)
    }

    /// Returns the sp1 host arguments for a rollup with celestia DA.
    #[cfg(feature = "celestia_da")]
    pub fn celestia_host_args() -> Arc<&'static [u8]> {
        if should_skip_guest_build() {
            return Arc::new(vec![].leak());
        }
        Arc::new(&sp1_prover::SP1_GUEST_CELESTIA_ELF)
    }

    /// Creates the inner VM from a prover config, returning the VM and the config discriminant.
    pub fn create_inner_vm(
        prover_config: sov_stf_runner::processes::RollupProverConfig<InnerZkvm>,
    ) -> (
        SP1Host<'static>,
        sov_stf_runner::processes::RollupProverConfigDiscriminants,
    ) {
        let (host_args, disc) = prover_config.split();
        (SP1Host::new(*host_args), disc)
    }
}

// Re-export the active inner ZKVM module
pub use inner::*;
