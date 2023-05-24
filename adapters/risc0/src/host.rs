use std::cell::RefCell;

use risc0_zkp::core::hash::sha::Sha256HashSuite;
// use risc0_zkp::core::hash::HashSuite;
// use risc0_zkp::field::baby_bear::BabyBear;
// use risc0_zkp::verify::{CpuVerifyHal, VerifyHal};
use risc0_zkp::verify::VerifyHal;
use risc0_zkvm::receipt::SessionReceipt;
use risc0_zkvm::{serde::to_vec, Executor, ExecutorEnv, ExecutorEnvBuilder};
use sov_rollup_interface::zk::traits::{Zkvm, ZkvmHost};

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
        let env = self.executor_env_builder.get_mut().build();
        let mut exec = Executor::from_elf(env, self.elf)?;
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
        let receipt: Risc0Proof<'a> = bincode::deserialize(serialized_proof)?;
        // verify_with_hal(
        //     &risc0_zkp::verify::CpuVerifyHal::<BabyBear, HashSuiteSha256<BabyBear, Impl>, _>::new(
        //                 &CIRCUIT,
        //     ),,
        //     &code_commitment.0,
        //     &receipt.seal,
        //     receipt.journal,
        // )?;
        // let a = CpuVerifyHal::<BabyBear, HashSuite<BabyBear>, _>::new(&CIRCUIT);
        // let a = CpuVerifyHal::<_, Sha256HashSuite<_, Impl>, _>::new(&CIRCUIT);
        // a.Ok(receipt.journal)
        Ok(serialized_proof)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Risc0Proof<'a> {
    pub journal: &'a [u8],
    pub seal: Vec<u32>,
}
