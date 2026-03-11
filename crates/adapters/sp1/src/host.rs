//! Implementation of the SP1 host for the Sovereign ZkvmHost trait.

use serde::Serialize;
use sov_rollup_interface::reexports::anyhow;
use sov_rollup_interface::zk::{Proof, ZkvmHost};
use sp1_sdk::blocking::{CpuProver, SP1PublicValues};
use sp1_sdk::blocking::{ProveRequest, Prover, ProverClient};
use sp1_sdk::{ProvingKey, SP1ProvingKey, SP1Stdin};

use crate::guest::SP1Guest;

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

    fn create_prover_and_pk(&self) -> anyhow::Result<(CpuProver, SP1ProvingKey)> {
        let prover = ProverClient::builder().cpu().build();
        let pk = prover
            .setup(self.elf.into())
            .map_err(|e| anyhow::anyhow!("SP1 setup failed. Error: {:?}", e))?;

        Ok((prover, pk))
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
        let proof: Proof<_, SP1PublicValues> = {
            if with_proof {
                let (prover, pk) = self.create_prover_and_pk()?;
                let output: sp1_sdk::SP1ProofWithPublicValues = prover
                    .prove(&pk, self.stdin.clone())
                    .compressed()
                    .run()
                    .map_err(|e| anyhow::anyhow!("SP1 proving failed. Error: {:?}", e))?;

                Proof::Full(output)
            } else {
                let prover = ProverClient::builder().mock().build();
                let output = prover.execute(self.elf.into(), self.stdin.clone()).run()?;
                Proof::PublicData(output.0)
            }
        };

        self.stdin = SP1Stdin::new();
        Ok(bincode::serialize(&proof)?)
    }

    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        let (_, pk) = self.create_prover_and_pk()?;
        Ok(crate::SP1MethodId(bincode::serialize(pk.verifying_key())?))
    }
}
