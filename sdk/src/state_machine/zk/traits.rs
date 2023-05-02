use core::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};

use crate::serial::{Decode, Encode};

/// A trait implemented by the prover ("host") of a zkVM program.
pub trait ZkvmHost: Zkvm {
    /// Give the guest a piece of advice non-deterministically
    fn write_to_guest<T: Encode>(&self, item: T);
}

/// A Zk proof system capable of proving and verifying arbitrary Rust code
/// Must support recursive proofs.
pub trait Zkvm {
    type CodeCommitment: Matches<Self::CodeCommitment> + Clone;
    type Proof: ProofTrait<Self>;
    type Error: Debug;

    fn verify(
        proof: Self::Proof,
        code_commitment: &Self::CodeCommitment,
    ) -> Result<<<Self as Zkvm>::Proof as ProofTrait<Self>>::Output, Self::Error>;
}


/// A trait which is acessible from within a zkVM program.
pub trait ZkvmGuest: Zkvm {
    /// Obtain "advice" non-deterministically from the host
    fn read_from_host<T: Decode>(&self) -> T;
}

/// A trait implemented by a zkVM proof.
pub trait ProofTrait<VM: Zkvm + ?Sized> {
    type Output: Encode + Decode;
    /// Verify the proof, deserializing the result if successful.
    fn verify(self, code_commitment: &VM::CodeCommitment) -> Result<Self::Output, VM::Error>;
}

pub trait Matches<T> {
    fn matches(&self, other: &T) -> bool;
}

pub enum RecursiveProofInput<Vm: Zkvm, T, Pf: ProofTrait<Vm, Output = T>> {
    Base(T),
    Recursive(Pf, std::marker::PhantomData<Vm>),
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct RecursiveProofOutput<Vm: Zkvm, T> {
    pub claimed_method_id: Vm::CodeCommitment,
    pub output: T,
}

// TODO!
mod risc0 {
    #[allow(unused)]
    struct MethodId([u8; 32]);
}
