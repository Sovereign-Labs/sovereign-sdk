//! This module implements the [`ZkvmHost`] trait for the RISC0 VM.

use risc0_zkvm::{ExecutorEnvBuilder, ExecutorImpl, InnerReceipt, Journal, Receipt, Session};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::zk::{Proof, Zkvm, ZkvmHost};

use crate::guest::Risc0Guest;
use crate::Risc0MethodId;

/// A [`Risc0Host`] stores a binary to execute in the Risc0 VM, and accumulates hints to be
/// provided to its execution.
#[derive(Clone)]
pub struct Risc0Host<'a> {
    env: Vec<u32>,
    elf: &'a [u8],
}

#[cfg(not(feature = "bench"))]
#[inline(always)]
fn add_benchmarking_callbacks(env: ExecutorEnvBuilder<'_>) -> ExecutorEnvBuilder<'_> {
    env
}

#[cfg(feature = "bench")]
fn add_benchmarking_callbacks(mut env: ExecutorEnvBuilder<'_>) -> ExecutorEnvBuilder<'_> {
    use sov_zk_cycle_utils::{cycle_count_callback, get_syscall_name, get_syscall_name_cycles};

    use crate::metrics::metrics_callback;

    let metrics_syscall_name = get_syscall_name();
    env.io_callback(metrics_syscall_name, metrics_callback);

    let cycles_syscall_name = get_syscall_name_cycles();
    env.io_callback(cycles_syscall_name, cycle_count_callback);

    env
}

impl<'a> Risc0Host<'a> {
    /// Create a new Risc0Host to prove the given binary.
    pub fn new(elf: &'a [u8]) -> Self {
        Self {
            env: Default::default(),
            elf,
        }
    }

    /// Run a computation in the zkVM without generating a receipt.
    /// This creates the "Session" trace without invoking the heavy cryptographic machinery.
    pub fn run_without_proving(&mut self) -> anyhow::Result<Session> {
        let env = add_benchmarking_callbacks(ExecutorEnvBuilder::default())
            .write_slice(&self.env)
            .build()
            .unwrap();
        let mut executor = ExecutorImpl::from_elf(env, self.elf)?;
        Ok(executor.run()?)
    }
    /// Run a computation in the zkvm and generate a receipt.
    pub fn run(&mut self) -> anyhow::Result<Receipt> {
        let session = self.run_without_proving()?;
        session.prove()
    }
}

impl<'a> ZkvmHost for Risc0Host<'a> {
    type Guest = Risc0Guest;

    fn add_hint<T: serde::Serialize>(&mut self, item: T) {
        // We use the in-memory size of `item` as an indication of how much
        // space to reserve. This is in no way guaranteed to be exact, but
        // usually the in-memory size and serialized data size are quite close.
        //
        // Note: this is just an optimization to avoid frequent reallocations,
        // it's not actually required.
        self.env
            .reserve(std::mem::size_of::<T>() / std::mem::size_of::<u32>());

        let mut serializer = risc0_zkvm::serde::Serializer::new(&mut self.env);
        item.serialize(&mut serializer)
            .expect("Risc0 hint serialization is infallible");
    }

    fn simulate_with_hints(&mut self) -> Self::Guest {
        Risc0Guest::with_hints(std::mem::take(&mut self.env))
    }

    fn run(&mut self, with_proof: bool) -> Result<Proof, anyhow::Error> {
        if with_proof {
            let receipt = self.run()?;
            let data = bincode::serialize(&receipt)?;
            Ok(Proof::Full(data))
        } else {
            let session = self.run_without_proving()?;
            let data = bincode::serialize(&session.journal)?;
            Ok(Proof::PublicInput(data))
        }
    }

    fn extract_output<Da: sov_rollup_interface::da::DaSpec, Root: Serialize + DeserializeOwned>(
        proof: &Proof,
    ) -> Result<sov_rollup_interface::zk::StateTransition<Da, Root>, Self::Error> {
        match proof {
            Proof::PublicInput(journal) => {
                let journal: Journal = bincode::deserialize(journal)?;
                Ok(journal.decode()?)
            }
            Proof::Full(data) => {
                let receipt: Receipt = bincode::deserialize(data)?;
                Ok(receipt.journal.decode()?)
            }
        }
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
        Da: sov_rollup_interface::da::DaSpec,
        Root: Serialize + DeserializeOwned,
    >(
        serialized_proof: &[u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<sov_rollup_interface::zk::StateTransition<Da, Root>, Self::Error> {
        let output = Self::verify(serialized_proof, code_commitment)?;
        Ok(risc0_zkvm::serde::from_slice(output)?)
    }
}

/// A verifier for Risc0 proofs.
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
        Da: sov_rollup_interface::da::DaSpec,
        Root: Serialize + DeserializeOwned,
    >(
        serialized_proof: &[u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<sov_rollup_interface::zk::StateTransition<Da, Root>, Self::Error> {
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
    /// The cryptographic data certifying the execution of the program.
    pub receipt: InnerReceipt,
    /// The public outputs produced by the program execution.
    pub journal: &'a [u8],
}
