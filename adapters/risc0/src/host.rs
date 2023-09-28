use std::sync::Mutex;

use risc0_zkvm::serde::to_vec;
use risc0_zkvm::{Executor, ExecutorEnvBuilder, InnerReceipt, Receipt, Session};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::zk::{Zkvm, ZkvmHost};
#[cfg(feature = "bench")]
use sov_zk_cycle_utils::{cycle_count_callback, get_syscall_name, get_syscall_name_cycles};

use crate::guest::Risc0Guest;
#[cfg(feature = "bench")]
use crate::metrics::metrics_callback;
use crate::Risc0MethodId;

pub struct Risc0Host<'a> {
    env: Mutex<Vec<u32>>,
    elf: &'a [u8],
}

#[cfg(not(feature = "bench"))]
#[inline(always)]
fn add_benchmarking_callbacks(env: ExecutorEnvBuilder<'_>) -> ExecutorEnvBuilder<'_> {
    env
}

#[cfg(feature = "bench")]
fn add_benchmarking_callbacks(mut env: ExecutorEnvBuilder<'_>) -> ExecutorEnvBuilder<'_> {
    let metrics_syscall_name = get_syscall_name();
    env.io_callback(metrics_syscall_name, metrics_callback);

    let cycles_syscall_name = get_syscall_name_cycles();
    env.io_callback(cycles_syscall_name, cycle_count_callback);

    env
}

impl<'a> Risc0Host<'a> {
    pub fn new(elf: &'a [u8]) -> Self {
        Self {
            env: Default::default(),
            elf,
        }
    }

    /// Run a computation in the zkvm without generating a receipt.
    /// This creates the "Session" trace without invoking the heavy cryptographic machinery.
    pub fn run_without_proving(&mut self) -> anyhow::Result<Session> {
        let env = add_benchmarking_callbacks(ExecutorEnvBuilder::default())
            .add_input(&self.env.lock().unwrap())
            .build()
            .unwrap();
        let mut executor = Executor::from_elf(env, self.elf)?;
        executor.run()
    }
    /// Run a computation in the zkvm and generate a receipt.
    pub fn run(&mut self) -> anyhow::Result<Receipt> {
        let session = self.run_without_proving()?;
        session.prove()
    }
}

impl<'a> ZkvmHost for Risc0Host<'a> {
    fn add_hint<T: serde::Serialize>(&self, item: T) {
        let serialized = to_vec(&item).expect("Serialization to vec is infallible");
        self.env.lock().unwrap().extend_from_slice(&serialized[..]);
    }

    type Guest = Risc0Guest;

    fn simulate_with_hints(&mut self) -> Self::Guest {
        Risc0Guest::with_hints(std::mem::take(&mut self.env.lock().unwrap()))
    }

    fn run(&mut self, with_proof: bool) -> Result<(), anyhow::Error> {
        if with_proof {
            self.run()?;
        } else {
            self.run_without_proving()?;
        }
        Ok(())
    }
}

impl<'host> Zkvm for Risc0Host<'host> {
    type CodeCommitment = Risc0MethodId;

    type Error = anyhow::Error;

    fn verify<'a>(
        serialized_proof: &'a [u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<&'a [u8], Self::Error> {
        verify_from_slice(serialized_proof, code_commitment)
    }

    fn verify_and_extract_output<
        Add: sov_rollup_interface::RollupAddress,
        Da: sov_rollup_interface::da::DaSpec,
        Root: Serialize + DeserializeOwned,
    >(
        serialized_proof: &[u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<sov_rollup_interface::zk::StateTransition<Da, Add, Root>, Self::Error> {
        let output = Self::verify(serialized_proof, code_commitment)?;
        Ok(risc0_zkvm::serde::from_slice(output)?)
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

    fn verify_and_extract_output<
        Add: sov_rollup_interface::RollupAddress,
        Da: sov_rollup_interface::da::DaSpec,
        Root: Serialize + DeserializeOwned,
    >(
        serialized_proof: &[u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<sov_rollup_interface::zk::StateTransition<Da, Add, Root>, Self::Error> {
        let output = Self::verify(serialized_proof, code_commitment)?;
        Ok(risc0_zkvm::serde::from_slice(output)?)
    }
}

fn verify_from_slice<'a>(
    serialized_proof: &'a [u8],
    code_commitment: &Risc0MethodId,
) -> Result<&'a [u8], anyhow::Error> {
    let Risc0Proof::<'a> {
        receipt, journal, ..
    } = bincode::deserialize(serialized_proof)?;

    receipt.verify(code_commitment.0, journal)?;
    Ok(journal)
}

/// A convenience type which contains the same data a Risc0 [`Receipt`] but borrows the journal
/// data. This allows us to avoid one unnecessary copy during proof verification.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Risc0Proof<'a> {
    pub receipt: InnerReceipt,
    pub journal: &'a [u8],
}
