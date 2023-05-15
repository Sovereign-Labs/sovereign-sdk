use core::fmt::Debug;

use serde::de::DeserializeOwned;
use serde::Serialize;

/// A trait implemented by the prover ("host") of a zkVM program.
pub trait ZkvmHost: Zkvm {
    /// Give the guest a piece of advice non-deterministically
    fn write_to_guest<T: Serialize>(&self, item: T);
}

/// A Zk proof system capable of proving and verifying arbitrary Rust code
/// Must support recursive proofs.
pub trait Zkvm {
    type CodeCommitment: Matches<Self::CodeCommitment>
        + Clone
        + Debug
        + Serialize
        + DeserializeOwned;
    type Error: Debug;

    /// Interpret a sequence of a bytes as a proof and attempt to verify it against the code commitment.
    /// If the proof is valid, return a reference to the public outputs of the proof.
    fn verify<'a>(
        serialized_proof: &'a [u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<&'a [u8], Self::Error>;
}

/// A trait which is accessible from within a zkVM program.
pub trait ZkvmGuest: Zkvm {
    /// Obtain "advice" non-deterministically from the host
    fn read_from_host<T: DeserializeOwned>(&self) -> T;
}

pub trait Matches<T> {
    fn matches(&self, other: &T) -> bool;
}

// TODO!
mod risc0 {
    #[allow(unused)]
    struct MethodId([u8; 32]);
}
