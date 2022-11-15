use core::fmt::Debug;

use serde::{Deserialize, Serialize};

use crate::env;

/// A proof that a program was executed in a zkVM.
pub trait ZkVM {
    type CodeCommitment: Matches<Self::CodeCommitment> + Clone;
    type Proof: Proof<Self>;
    type Error: Debug;

    fn log<T: serde::Serialize>();
    fn verify(
        proof: Self::Proof,
        code_commitment: &Self::CodeCommitment,
    ) -> Result<<<Self as ZkVM>::Proof as Proof<Self>>::Output, Self::Error>;
}

pub trait Proof<VM: ZkVM + ?Sized> {
    type Output: Serialize + Deserialize;
    /// Verify the proof, deserializing the result if successful.
    fn verify(self, code_commitment: &VM::CodeCommitment) -> Result<Self::Output, VM::Error>;
}

pub trait Matches<T> {
    fn matches(&self, other: &T) -> bool;
}

pub enum RecursiveProofInput<Vm: ZkVM, T, Pf: Proof<Vm, Output = T>> {
    Base(T),
    Recursive(Pf, std::marker::PhantomData<Vm>),
}

pub struct RecursiveProofOutput<Vm: ZkVM, T> {
    pub claimed_method_id: Vm::CodeCommitment,
    pub output: T,
}
impl<Vm: ZkVM, T> Serialize for RecursiveProofOutput<Vm, T> {}
impl<Vm: ZkVM, T> Deserialize for RecursiveProofOutput<Vm, T> {}

// TODO!
mod risc0 {
    struct MethodId([u8; 32]);
}

// TODO!
pub mod serde {
    pub trait Serialize {}
    pub trait Deserialize {}
}
