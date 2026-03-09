#![allow(dead_code)]
//! Implementation of the SP1 host for the Sovereign ZkvmHost trait.

use serde::Serialize;
use sov_rollup_interface::reexports::anyhow;
use sov_rollup_interface::zk::{Proof, ZkvmHost};
use sp1_sdk::blocking::{ProveRequest, Prover, ProverClient};
use sp1_sdk::{ProvingKey, SP1PublicValues, SP1Stdin};

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

    /// TODO
    pub async fn run_async(&mut self, _with_proof: bool) -> anyhow::Result<Vec<u8>> {
        use sp1_sdk::{ProveRequest, Prover, ProverClient}; // async API

        if cfg!(debug_assertions) {
            //std::env::set_var("SP1_PROVER", "mock");
            std::env::set_var("SP1_PROVER", "cpu");
        } else {
            std::env::set_var("SP1_PROVER", "cpu");
        }
        //let prover = ProverClient::from_env().await;
        let prover = ProverClient::builder().cpu().build().await;
        let proof: Proof<_, SP1PublicValues> = {
            let pk = prover
                .setup(self.elf.into())
                .await
                .map_err(|e| anyhow::anyhow!("SP1 setup failed. Error: {:?}", e))?;
            let output = prover
                .prove(&pk, self.stdin.clone())
                .compressed()
                .await
                .map_err(|e| anyhow::anyhow!("SP1 proving failed. Error: {:?}", e))?;

            Proof::Full(output)
        };

        self.stdin = SP1Stdin::new();

        Ok(bincode::serialize(&proof)?)
    }

    /// Returns a commitment to the guest ELF using SP1's async prover API.
    pub async fn code_commitment_async(&self) -> anyhow::Result<crate::SP1MethodId> {
        use sp1_sdk::{Prover, ProverClient};

        let prover = ProverClient::builder().cpu().build().await;
        let pk = prover
            .setup(self.elf.into())
            .await
            .map_err(|e| anyhow::anyhow!("SP1 setup failed. Error: {:?}", e))?;

        Ok(crate::SP1MethodId(bincode::serialize(pk.verifying_key())?))
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
        if cfg!(debug_assertions) {
            //std::env::set_var("SP1_PROVER", "mock");
            std::env::set_var("SP1_PROVER", "cpu");
        } else {
            std::env::set_var("SP1_PROVER", "cpu");
        }
        let prover = ProverClient::from_env();
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
            let prover = ProverClient::builder().cpu().build();
            let execute_request = prover.execute(self.elf.into(), self.stdin.clone());
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
        crate::SP1MethodId(bincode::serialize(pk.verifying_key()).unwrap())
    }
}
