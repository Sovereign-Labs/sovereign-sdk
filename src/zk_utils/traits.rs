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

pub trait RecursiveProof {
    type Vm: ZkVM;
    type InOut;
    type Error;
    fn verify_base(input: Self::InOut) -> Result<(), Self::Error>;
    fn verify_continuity(previous: Self::InOut) -> Result<(), Self::Error>;
    fn work(input: Self::InOut) -> RecursiveProofOutput<Self::Vm, Self::InOut>;
}

// pub fn recursify<Vm, T, Pf: Proof<Vm, Output = T>>() {
//     let input: RecursiveProofInput<T, Pf> = env::read_unchecked();
//     match input {
//         RecursiveProofInput::Base(b) => todo!(),
//         RecursiveProofInput::Recursive(proof) => proof.ver,
//     }
// }

// pub struct RecursiveProof<Vm: ZkVM, T> {
//     output: RecursiveProofOutput<Vm, T>,
// }
// impl<Vm: ZkVM<Proof = Self>, T> Proof<Vm> for RecursiveProof<Vm, T> {
//     type Output = RecursiveProofOutput<Vm, T>;
//     fn verify(self, code_commitment: &Vm::CodeCommitment) -> Result<Self::Output, Vm::Error> {
//         Vm::verify(self, code_commitment)
//     }
// }
// impl<Vm: ZkVM<Proof = Self>, T> RecursiveProof<Vm, T> {
//     pub fn recurse(&mut self, previous_instance: Self, code_commitment: Vm::CodeCommitment) -> T {
//         let prev_output = previous_instance
//             .verify(&code_commitment)
//             .expect("proof must be valid");
//         assert!(code_commitment.matches(&prev_output.claimed_method_id));

//         self.output.claimed_method_id = code_commitment;
//         prev_output.output
//     }

//     pub fn set_output(&mut self, output: T) {
//         self.output.output = output;
//     }
// }

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
