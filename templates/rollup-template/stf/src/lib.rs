//! The rollup State Transition Function.
#[cfg(feature = "native")]
mod builder;
#[cfg(feature = "native")]
mod genesis_config;
mod hooks;
mod runtime;

#[cfg(feature = "native")]
pub use builder::*;
#[cfg(feature = "native")]
pub use genesis_config::*;
pub use runtime::*;

use sov_rollup_interface::da::DaVerifier as _;
use sov_rollup_interface::mocks::MockDaVerifier;
use sov_risc0_adapter::guest::Risc0Guest;

/// The type alias for the DA layer verifier. Change the contents of this alias if you change DA layers.
pub type DaVerifier = MockDaVerifier;

/// The type alias for the guest ("verifier").
pub type ZkvmGuest = Risc0Guest;

/// Creates a new verifier for the rollup's DA.
pub fn new_da_verifier() -> DaVerifier {
    DaVerifier::new(())
}
