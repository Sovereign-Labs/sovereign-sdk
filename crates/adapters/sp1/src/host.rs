//! Implementation of the SP1 host for the Sovereign ZkvmHost trait.

use crate::guest::SP1Guest;
use crate::SP1MethodId;
use crate::ZkVerifier;
use serde::Serialize;
use sov_rollup_interface::reexports::anyhow;
use sov_rollup_interface::zk::aggregated_proof::ZkvmHostWithInnerProofs;
use sov_rollup_interface::zk::ZkvmGuest;
use sov_rollup_interface::zk::{Proof, ZkvmHost};
use sp1_sdk::blocking::CpuProver;
use sp1_sdk::blocking::{ProveRequest, Prover, ProverClient};
use sp1_sdk::{ProvingKey, SP1Proof, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin};

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

    /// Adds a compressed SP1 proof and its verifying key to the host's stdin
    /// so it can be verified inside the guest program during aggregation.
    pub fn add_proof_inner(
        &mut self,
        proof: &SP1ProofWithPublicValues,
        method_id: &SP1MethodId,
    ) -> anyhow::Result<()> {
        let SP1Proof::Compressed(recursion_proof) = &proof.proof else {
            anyhow::bail!("Expected a compressed SP1 proof");
        };
        let vk: sp1_sdk::SP1VerifyingKey = bincode::deserialize(&method_id.0)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize SP1VerifyingKey: {e}"))?;

        self.stdin
            .write_proof(*recursion_proof.clone(), vk.vk.clone());
        Ok(())
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
        let stdin = std::mem::take(&mut self.stdin);

        let result = if with_proof {
            let (prover, pk) = self.create_prover_and_pk()?;
            let output: sp1_sdk::SP1ProofWithPublicValues = prover
                .prove(&pk, stdin)
                .compressed()
                .run()
                .map_err(|e| anyhow::anyhow!("SP1 proving failed. Error: {:?}", e))?;

            Proof::Full(output)
        } else {
            let prover = ProverClient::builder().mock().build();
            let output = prover.execute(self.elf.into(), stdin).run()?;
            Proof::PublicData(output.0)
        };

        Ok(bincode::serialize(&result)?)
    }

    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        let (_, pk) = self.create_prover_and_pk()?;
        Ok(crate::SP1MethodId(bincode::serialize(pk.verifying_key())?))
    }
}

impl ZkvmHostWithInnerProofs for SP1Host<'static> {
    type Proof = SP1ProofWithPublicValues;

    fn add_proof(
        &mut self,
        proof: &Self::Proof,
        code_commitment: &<<Self::Guest as ZkvmGuest>::Verifier as ZkVerifier>::CodeCommitment,
    ) -> anyhow::Result<()> {
        self.add_proof_inner(proof, code_commitment)
    }
}
