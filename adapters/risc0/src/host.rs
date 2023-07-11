use std::cell::RefCell;

// use risc0_zkp::core::config::HashSuiteSha256;
// use risc0_zkp::core::hash::sha::Sha256HashSuite;
// use risc0_zkp::field::baby_bear::BabyBear;
// use risc0_zkp::core::hash::HashSuite;
// use risc0_zkp::field::baby_bear::BabyBear;
// use risc0_zkp::verify::{CpuVerifyHal, VerifyHal};
// use risc0_zkp::verify::VerifyHal;
// use risc0_zkvm::receipt::{verify_with_hal, SessionReceipt};
use risc0_zkvm::receipt::SessionReceipt;
use risc0_zkvm::serde::to_vec;
// use risc0_zkvm::sha::Impl;
use risc0_zkvm::{
    Executor, ExecutorEnv, ExecutorEnvBuilder, LocalExecutor, MemoryImage, Program, MEM_SIZE,
    PAGE_SIZE,
};
use sov_rollup_interface::zk::{Zkvm, ZkvmHost};

use crate::Risc0MethodId;

const CIRCUIT: risc0_circuit_rv32im::CircuitImpl = risc0_circuit_rv32im::CircuitImpl::new();

pub struct Risc0Host<'a> {
    executor_env_builder: RefCell<ExecutorEnvBuilder<'a>>,
    elf: &'a [u8],
}

impl<'a> Risc0Host<'a> {
    pub fn new(elf: &'a [u8]) -> Self {
        Self {
            executor_env_builder: RefCell::new(ExecutorEnv::builder()),
            elf,
        }
    }

    pub fn run(&mut self) -> anyhow::Result<SessionReceipt> {
        let env = self.executor_env_builder.get_mut().build().unwrap();
        let program = Program::load_elf(self.elf, MEM_SIZE as u32)?;
        let image = MemoryImage::new(&program, PAGE_SIZE as u32)?;
        let mut exec = LocalExecutor::new(env, image, program.entry);
        let session = exec.run()?;
        session.prove()
    }
}

impl<'a> ZkvmHost for Risc0Host<'a> {
    fn write_to_guest<T: serde::Serialize>(&self, item: T) {
        let serialized = to_vec(&item).expect("Serialization to vec is infallible");
        self.executor_env_builder
            .borrow_mut()
            .add_input(&serialized);
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
    let receipt: SessionReceipt = bincode::deserialize(serialized_proof)?;
    // let receipt: Risc0Proof<'a> = bincode::deserialize(serialized_proof)?;

    // verify_with_hal(
    //     &risc0_zkp::verify::CpuVerifyHal::<BabyBear, HashSuiteSha256<BabyBear, Impl>, _>::new(
    //         &CIRCUIT,
    //     ),
    //     &code_commitment.0,
    //     &receipt.seal,
    //     receipt.journal,
    // )?;
    Ok(&receipt.journal)
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Risc0Proof<'a> {
    pub journal: &'a [u8],
    pub seal: Vec<u32>,
}
