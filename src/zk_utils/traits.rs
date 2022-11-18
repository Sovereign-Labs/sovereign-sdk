use core::fmt::Debug;

use bytes::Bytes;
use serial::{Deserialize, Serialize};

use crate::env;

/// A proof that a program was executed in a zkVM.
pub trait ZkVM {
    type CodeCommitment: Matches<Self::CodeCommitment> + Clone;
    type Proof: Proof<Self>;
    type Error: Debug;

    fn log<T: serial::Serialize>(item: T);
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
impl<Vm: ZkVM<CodeCommitment = C>, C: Serialize, T: Serialize> Serialize
    for RecursiveProofOutput<Vm, T>
{
    fn serialize(&self, target: &mut Vec<u8>) {
        self.claimed_method_id.serialize(target);
        self.output.serialize(target);
    }
}
impl<Vm: ZkVM, T> Deserialize for RecursiveProofOutput<Vm, T> {
    fn deserialze(target: &mut &[u8]) -> Result<Self, serial::DeserializationError> {
        todo!()
    }
}

// TODO!
mod risc0 {
    struct MethodId([u8; 32]);
}

// TODO!
pub mod serial {
    pub enum DeserializationError {
        DataTooShort,
    }

    // TODO: do this in a sensible/generic way
    // The objective is to not introduce a forcible serde dependency and potentially
    // allow implementers to use rykv or another zero-copy framework. But we
    // need to design that. This will work for now
    pub trait Serialize {
        fn serialize(&self, target: &mut Vec<u8>);
        fn serialize_to_vec(&self) -> Vec<u8> {
            let mut target = Vec::new();
            self.serialize(&mut target);
            target
        }
    }

    // impl<T: Serialize> Serialize for &T {
    //     fn serialize(&self, target: &mut Vec<u8>) {
    //         (*self).serialize(target);
    //     }
    // }
    pub trait Deserialize: Sized {
        fn deserialze(target: &mut &[u8]) -> Result<Self, DeserializationError>;
    }
}
