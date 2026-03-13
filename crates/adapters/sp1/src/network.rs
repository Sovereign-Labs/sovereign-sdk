//! Network prover implementation for SP1.
//!
//! Submits proof requests to the Succinct proving network and polls for results.

use serde::Serialize;
use sov_rollup_interface::zk::{Proof, ZkvmNetwork};
use sp1_sdk::network::B256;
use sp1_sdk::network::proto::base_types::FulfillmentStatus;
use sp1_sdk::prover::{Prover, ProveRequest};
use sp1_sdk::{NetworkProver, SP1ProvingKey, SP1Stdin};

use crate::guest::SP1Guest;

pub type ProofHandle = B256;

/// SP1 network prover that submits proofs to the Succinct proving network.
pub struct SP1Network {
    prover: NetworkProver,
    pk: SP1ProvingKey,
    stdin: SP1Stdin,
}

impl SP1Network {
    /// Create a new `SP1Network` from an existing [`NetworkProver`] and an ELF binary.
    pub async fn new(prover: NetworkProver, elf: &[u8]) -> anyhow::Result<Self> {
        let pk = prover
            .setup(elf.into())
            .await
            .map_err(|e| anyhow::anyhow!("SP1 network setup failed: {e}"))?;

        Ok(Self {
            prover,
            pk,
            stdin: SP1Stdin::new(),
        })
    }
}

impl ZkvmNetwork for SP1Network {
    type Guest = SP1Guest;
    type ProofHandle = ProofHandle;

    fn add_hint<T: Serialize>(&mut self, item: &T) {
        self.stdin.write(item);
    }

    async fn submit(&mut self) -> anyhow::Result<Self::ProofHandle> {
        let request_id = self
            .prover
            .prove(&self.pk, self.stdin.clone())
            .compressed()
            .skip_simulation(true)
            .request()
            .await?;

        self.stdin = SP1Stdin::new();
        Ok(request_id)
    }

    async fn poll(&self, handle: &Self::ProofHandle) -> anyhow::Result<Option<Vec<u8>>> {
        let (maybe_proof, status) = self.prover.process_proof_status(*handle, None).await?;

        if matches!(status, FulfillmentStatus::Unfulfillable) {
            anyhow::bail!("Proof request {} is unfulfillable", handle);
        }

        match maybe_proof {
            Some(proof) => Ok(Some(bincode::serialize(&Proof::<_, sp1_sdk::SP1PublicValues>::Full(proof))?)),
            None => Ok(None),
        }
    }
}
