//! Implementation of the SP1 host for the Sovereign ZkvmHost trait.

use serde::Serialize;
use sov_rollup_interface::reexports::anyhow;
use sov_rollup_interface::zk::{Proof, ZkvmHost};
use sp1_sdk::{HookEnv, Prover, ProverClient, SP1Stdin};

use crate::guest::SP1Guest;

/// Necessary since similar functionality on `Prove` and `Execute` do not yet come from a shared trait.
#[allow(dead_code)]
trait WithHook<'a> {
    fn with_hook(
        self,
        fd: u32,
        f: impl FnMut(HookEnv, &[u8]) -> Vec<Vec<u8>> + Send + Sync + 'a,
    ) -> Self;
}

// Note: CI didn't like this, so we're just going to ignore it.
#[allow(dead_code)]
impl<'a> WithHook<'a> for sp1_sdk::cpu::execute::CpuExecuteBuilder<'a> {
    fn with_hook(
        self,
        fd: u32,
        f: impl FnMut(HookEnv, &[u8]) -> Vec<Vec<u8>> + Send + Sync + 'a,
    ) -> Self {
        self.with_hook(fd, f)
    }
}

#[cfg(feature = "bench")]
fn cycle_count_hook(env: HookEnv, _buf: &[u8]) -> Vec<Vec<u8>> {
    vec![Vec::from(
        env.runtime.report.total_instruction_count().to_le_bytes(),
    )]
}

#[cfg(feature = "bench")]
fn add_benchmarking_hooks<'a, T: WithHook<'a>>(action_builder: T) -> T {
    use sov_metrics::cycle_utils::sp1::{FD_CYCLE_COUNT_HOOK, FD_METRICS_HOOK};

    use crate::metrics::metrics_hook;

    action_builder
        .with_hook(FD_CYCLE_COUNT_HOOK, cycle_count_hook)
        .with_hook(FD_METRICS_HOOK, metrics_hook)
}

#[cfg(not(feature = "bench"))]
fn add_benchmarking_hooks<'a, T: WithHook<'a>>(action_builder: T) -> T {
    action_builder
}

/// SP1 Host implementation.
pub struct SP1Host<'host> {
    elf: &'host [u8],
    stdin: SP1Stdin,
}

/// Instantiate a new SP1 Host.
impl<'host> SP1Host<'host> {
    /// Create a new SP1 Host.
    pub fn new(elf: &'host [u8]) -> Self {
        Self {
            elf,
            stdin: SP1Stdin::new(),
        }
    }

    /// Create a new `Sp1Guest` that reads the provided hints
    pub fn simulate_with_hints(&mut self) -> SP1Guest {
        SP1Guest::with_hints(self.stdin.buffer.clone())
    }
}

impl Clone for SP1Host<'_> {
    fn clone(&self) -> Self {
        Self {
            elf: self.elf,
            stdin: self.stdin.clone(),
        }
    }
}

impl core::fmt::Debug for SP1Host<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1Host").finish()
    }
}

impl ZkvmHost for SP1Host<'static> {
    type HostArgs = &'static [u8];
    type Guest = SP1Guest;

    fn from_args(args: &Self::HostArgs) -> Self {
        Self::new(args)
    }

    fn add_hint<T: Serialize>(&mut self, item: T) {
        self.stdin.write(&item);
    }

    fn run(&mut self, with_proof: bool) -> anyhow::Result<Vec<u8>> {
        let prover = if cfg!(debug_assertions) {
            ProverClient::builder().mock().build()
        } else {
            ProverClient::builder().cpu().build()
        };
        let proof = if with_proof {
            let (pk, _) = prover.setup(self.elf);
            // Bbenchmarking hooks only available for `CpuExecuteBuilder`, not `CpuProverBuilder`
            let output = prover
                .prove(&pk, &self.stdin)
                .run()
                .map_err(|e| anyhow::anyhow!("SP1 proving failed. Error: {:?}", e))?;
            Proof::Full(output.proof)
        } else {
            let prover = ProverClient::builder().mock().build();
            let output = add_benchmarking_hooks(prover.execute(self.elf, &self.stdin))
                .run()
                .map_err(|e| anyhow::anyhow!("SP1 execution failed. Error: {:?}", e))?;
            Proof::PublicData(output.0)
        };
        Ok(bincode::serialize(&proof)?)
    }

    fn code_commitment(&self) -> <<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment{
        let verifying_key = ProverClient::from_env().setup(self.elf).1;
        crate::SP1MethodId(bincode::serialize(&verifying_key).unwrap())
    }
}
