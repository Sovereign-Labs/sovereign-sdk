//! Implementation of the SP1 host for the Sovereign ZkvmHost trait.

use serde::Serialize;
use sov_rollup_interface::reexports::anyhow;
use sov_rollup_interface::zk::{Proof, ZkvmHost};
use sp1_sdk::blocking::{Prover, ProverClient};
use sp1_sdk::{HookEnv, SP1Stdin};

use crate::guest::SP1Guest;

#[cfg(feature = "bench")]
fn cycle_count_hook(_env: HookEnv, _buf: &[u8]) -> Vec<Vec<u8>> {
    // TODO: HookEnv is an empty struct in V6, so we can't access runtime.report.
    // Return 0 as a placeholder — benchmarking is not correctness-critical.
    vec![Vec::from(0u64.to_le_bytes())]
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
            let pk = prover
                .setup(self.elf.into())
                .map_err(|e| anyhow::anyhow!("SP1 setup failed. Error: {:?}", e))?;
            let output = prover
                .prove(&pk, self.stdin.clone())
                .run()
                .map_err(|e| anyhow::anyhow!("SP1 proving failed. Error: {:?}", e))?;
            Proof::Full(output.proof)
        } else {
            let prover = ProverClient::builder().mock().build();
            let mut execute_request = prover.execute(self.elf.into(), self.stdin.clone());
            #[cfg(feature = "bench")]
            {
                use sov_metrics::cycle_utils::sp1::{FD_CYCLE_COUNT_HOOK, FD_METRICS_HOOK};

                use crate::metrics::metrics_hook;

                execute_request = execute_request
                    .with_hook(FD_CYCLE_COUNT_HOOK, cycle_count_hook)
                    .with_hook(FD_METRICS_HOOK, metrics_hook);
            }
            let (public_values, _report) = execute_request
                .run()
                .map_err(|e| anyhow::anyhow!("SP1 execution failed. Error: {:?}", e))?;
            Proof::PublicData(public_values)
        };
        Ok(bincode::serialize(&proof)?)
    }

    fn code_commitment(&self) -> <<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment{
        let pk = sp1_sdk::blocking::ProverClient::from_env()
            .setup(self.elf.into())
            .expect("SP1 setup failed");
        crate::SP1MethodId(bincode::serialize(&pk.vk).unwrap())
    }
}
