//! Network prover implementation for SP1.
//!
//! Submits proof requests to the Succinct proving network and polls for results.

use serde::Serialize;
use sov_rollup_interface::zk::{ZkVerifier, ZkvmGuest, ZkvmNetwork};
use sp1_sdk::network::proto::auction_types::FulfillmentStatus;
use sp1_sdk::network::{NetworkMode, B256};
use sp1_sdk::prover::{ProveRequest, Prover};
use sp1_sdk::{NetworkProver, ProverClient, ProvingKey, SP1ProvingKey, SP1Stdin};

use crate::guest::SP1Guest;

/// Re-export of the proof handle type used by the SP1 network.
pub type ProofHandle = B256;

/// SP1 network prover that submits proofs to the Succinct proving network.
pub struct SP1Network {
    prover: NetworkProver,
    pk: SP1ProvingKey,
}

impl SP1Network {
    /// Create a new `SP1Network` for the given ELF binary.
    ///
    /// Connects to the Succinct proving network (mainnet) and runs setup.
    pub async fn new(elf: &[u8]) -> anyhow::Result<Self> {
        let prover: NetworkProver = ProverClient::builder()
            .network_for(NetworkMode::Mainnet)
            .build()
            .await;
        let pk = prover
            .setup(elf.into())
            .await
            .map_err(|e| anyhow::anyhow!("SP1 network setup failed: {e}"))?;

        Ok(Self { prover, pk })
    }
}

impl ZkvmNetwork for SP1Network {
    type Guest = SP1Guest;
    type ProofHandle = ProofHandle;

    async fn add_hint_and_submit<T: Serialize + Send + Sync>(
        &self,
        item: &T,
    ) -> anyhow::Result<Self::ProofHandle> {
        let mut stdin = SP1Stdin::new();
        stdin.write(item);

        let request_id = self
            .prover
            .prove(&self.pk, stdin)
            .compressed()
            .skip_simulation(true)
            .request()
            .await?;

        Ok(request_id)
    }

    async fn poll(&self, handle: &Self::ProofHandle) -> anyhow::Result<Option<Vec<u8>>> {
        let (maybe_proof, status) = self.prover.process_proof_status(*handle, None).await?;

        if matches!(status, FulfillmentStatus::Unfulfillable) {
            anyhow::bail!("Proof request {} is unfulfillable", handle);
        }

        match maybe_proof {
            Some(proof) => Ok(Some(bincode::serialize(&proof)?)),
            None => Ok(None),
        }
    }

    fn code_commitment(
        &self,
    ) -> anyhow::Result<<<Self::Guest as ZkvmGuest>::Verifier as ZkVerifier>::CodeCommitment> {
        Ok(crate::SP1MethodId(bincode::serialize(
            self.pk.verifying_key(),
        )?))
    }
}
