//! This module implements the [`ZkvmHost`] trait for the RISC0 VM.

use crate::guest::Risc0Guest;
use crate::Risc0MethodId;
use risc0_zkvm::{ExecutorEnvBuilder, ExecutorImpl, Session};
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::aggregated_proof::BlockProof;
use sov_rollup_interface::zk::aggregated_proof::OuterZkvmHost;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_rollup_interface::zk::SerializedZkProof;
use sov_rollup_interface::zk::ZkvmHost;

/// A [`Risc0Host`] stores a binary to execute in the Risc0 VM, and accumulates hints to be
/// provided to its execution.
#[derive(Clone)]
pub struct Risc0Host<'a> {
    #[cfg(feature = "bincode")]
    env: Vec<u8>,
    #[cfg(not(feature = "bincode"))]
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
    use crate::metrics::{metrics_callback, SYSCALL_NAME_METRICS};

    env.io_callback(SYSCALL_NAME_METRICS, metrics_callback);

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
    fn run_without_proving(&mut self) -> anyhow::Result<Session> {
        let mut env = add_benchmarking_callbacks(ExecutorEnvBuilder::default());
        #[cfg(feature = "bincode")]
        env.write_slice(&[self.env.len() as u32]);
        let env = env.write_slice(&self.env).build().unwrap();
        self.env.clear();
        let mut executor = ExecutorImpl::from_elf(env, self.elf)?;
        executor.run()
    }

    fn replace_hints<T: serde::Serialize>(&mut self, item: &T) {
        self.env.clear();
        self.add_hint(item);
    }

    /// Push a non-deterministic hint into the zkvm environment.
    pub fn add_hint<T: serde::Serialize>(&mut self, item: &T) {
        // We use the in-memory size of `item` as an indication of how much
        // space to reserve. This is in no way guaranteed to be exact, but
        // usually the in-memory size and serialized data size are quite close.
        //
        // Note: this is just an optimization to avoid frequent reallocation,
        // it's not actually required.
        self.env
            .reserve(std::mem::size_of::<T>() / std::mem::size_of::<u32>());

        #[cfg(not(feature = "bincode"))]
        {
            let mut serializer = risc0_zkvm::serde::Serializer::new(&mut self.env);
            item.serialize(&mut serializer)
                .expect("Risc0 hint serialization is infallible");
        }

        #[cfg(feature = "bincode")]
        bincode::serialize_into(&mut self.env, item)
            .expect("Risc0 hint serialization is infallible");
    }

    /// Generate a Risc0Guest with provided hints
    pub fn simulate_with_hints(&mut self) -> Risc0Guest {
        Risc0Guest::with_hints(std::mem::take(&mut self.env))
    }
}

impl ZkvmHost for Risc0Host<'static> {
    type Guest = Risc0Guest;

    fn add_hint_deferred_and_run<T: Serialize>(
        &mut self,
        item: &T,
        _agg_proofs: Vec<SerializedAggregatedProof>,
    ) -> anyhow::Result<SerializedZkProof> {
        self.replace_hints(item);
        let session = self.run_without_proving()?;
        let receipt = session.prove()?.receipt;
        Ok(SerializedZkProof {
            raw_proof: bincode::serialize(&receipt)?,
        })
    }

    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        Ok(Risc0MethodId(
            risc0_zkvm::compute_image_id(self.elf)?.into(),
        ))
    }
}

impl OuterZkvmHost for Risc0Host<'static> {
    fn run_proof_aggregation<Address: Serialize + Clone, Da: DaSpec, Root: Serialize + Clone>(
        &self,
        _genesis_state_root: Root,
        _headers_with_block_proofs: Vec<(Da::BlockHeader, BlockProof<Address, Da, Root>)>,
    ) -> anyhow::Result<SerializedAggregatedProof> {
        unimplemented!("Proof aggregation not supported for Risc0")
    }
}
