use std::cell::RefCell;

use risc0_zkp::core::config::HashSuiteSha256;
use risc0_zkp::field::baby_bear::BabyBear;
use risc0_zkvm::receipt::verify_with_hal;
use risc0_zkvm::serde::to_vec;
use risc0_zkvm::sha::Impl;
use risc0_zkvm::{Prover, Receipt};
use sov_rollup_interface::zk::traits::{Zkvm, ZkvmHost};

use crate::Risc0MethodId;

const CIRCUIT: risc0_circuit_rv32im::CircuitImpl = risc0_circuit_rv32im::CircuitImpl::new();

pub struct Risc0Host<'a> {
    prover: RefCell<Prover<'a>>,
}

impl<'a> Risc0Host<'a> {
    pub fn new(elf: &'a [u8]) -> Self {
        Self {
            prover: RefCell::new(
                Prover::new(elf).expect("Prover should be constructed from valid ELF binary"),
            ),
        }
    }

    pub fn run(&mut self) -> anyhow::Result<Receipt> {
        self.prover.borrow_mut().run()
    }
}

impl<'a> ZkvmHost for Risc0Host<'a> {
    fn write_to_guest<T: serde::Serialize>(&self, item: T) {
        let serialized = to_vec(&item).expect("Serialization to vec is infallible");
        self.prover.borrow_mut().add_input_u32_slice(&serialized);
    }
}

impl<'prover> Zkvm for Risc0Host<'prover> {
    type CodeCommitment = Risc0MethodId;

    type Error = anyhow::Error;

    fn verify<'a>(
        serialized_proof: &'a [u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<&'a [u8], Self::Error> {
        verify_from_slice(serialized_proof, code_commitment)
    }
}

pub struct Risc0Verifier;

impl Zkvm for Risc0Verifier {
    type CodeCommitment = Risc0MethodId;

    type Error = anyhow::Error;

    fn verify<'a>(
        serialized_proof: &'a [u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<&'a [u8], Self::Error> {
        verify_from_slice(serialized_proof, code_commitment)
    }
}

fn verify_from_slice<'a>(
    serialized_proof: &'a [u8],
    code_commitment: &Risc0MethodId,
) -> Result<&'a [u8], anyhow::Error> {
    let receipt: Risc0Proof<'a> = bincode::deserialize(serialized_proof)?;
    verify_with_hal(
        &risc0_zkp::verify::CpuVerifyHal::<BabyBear, HashSuiteSha256<BabyBear, Impl>, _>::new(
            &CIRCUIT,
        ),
        &code_commitment.0,
        &receipt.seal,
        receipt.journal,
    )?;
    Ok(receipt.journal)
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Risc0Proof<'a> {
    pub journal: &'a [u8],
    pub seal: Vec<u32>,
}
