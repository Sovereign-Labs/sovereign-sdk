//! Defines mock instantiations of many important traits, which are useful
//! for testing, fuzzing, and benchmarking.

mod da;
#[cfg(feature = "native")]
mod service;
#[cfg(feature = "std")]
mod use_std;
mod validity_condition;
mod zk_vm;
pub use da::{
    MockAddress, MockBlockHeader, MockDaConfig, MockDaSpec, MockDaVerifier, MockHash,
    MOCK_SEQUENCER_DA_ADDRESS,
};
#[cfg(feature = "native")]
pub use service::MockDaService;
#[cfg(feature = "std")]
pub use use_std::{MockBlob, MockBlock};
pub use validity_condition::{MockValidityCond, MockValidityCondChecker};
pub use zk_vm::{MockCodeCommitment, MockProof, MockZkvm};
