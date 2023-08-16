//! Defines mock instantiations of many important traits, which are useful
//! for testing, fuzzing, and benchmarking.

mod da;
mod validity_condition;
mod zk_vm;
pub use da::{
    MockAddress, MockBatchBuilder, MockBlob, MockBlock, MockBlockHeader, MockDaService, MockDaSpec,
    MockHash,
};
pub use validity_condition::MockValidityCond;
pub use zk_vm::{MockCodeCommitment, MockProof, MockZkvm};
