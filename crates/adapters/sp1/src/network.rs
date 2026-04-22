//! Network prover implementation for SP1.
//!
//! Submits proof requests to the Succinct proving network and polls for results.

use crate::guest::SP1Guest;
use serde::Serialize;
use sov_rollup_interface::zk::{ZkVerifier, ZkvmGuest, ZkvmNetwork};
use sp1_sdk::network::proto::auction_types::FulfillmentStatus;
use sp1_sdk::network::{NetworkMode, B256};
use sp1_sdk::prover::{ProveRequest, Prover};
use sp1_sdk::HashableKey;
use sp1_sdk::{NetworkProver, ProverClient, ProvingKey, SP1ProvingKey, SP1Stdin};

#[cfg(feature = "metrics")]
mod metrics {
    use std::io::Write;

    use sov_metrics::Metric;

    /// Metrics emitted when the SP1 proving network fulfills a proof request.
    #[derive(Debug)]
    pub(super) struct SP1ProofFulfillmentMetrics {
        /// The hex-encoded proof request ID.
        pub request_id: String,
        /// Time from proof request creation to fulfillment, in seconds, as reported by the SP1
        /// network.
        pub fulfillment_duration_secs: u64,
    }

    impl Metric for SP1ProofFulfillmentMetrics {
        fn measurement_name(&self) -> &'static str {
            "sov_rollup_sp1_proof_fulfillment"
        }

        fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
            write!(
                buffer,
                "{} request_id=\"{}\",fulfillment_duration_secs={}i",
                self.measurement_name(),
                self.request_id,
                self.fulfillment_duration_secs,
            )
        }
    }
}

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

#[cfg(feature = "metrics")]
impl SP1Network {
    async fn emit_fulfillment_metric(&self, request_id: B256) {
        let proof_request = match self.prover.get_proof_request(request_id).await {
            Ok(Some(req)) => req,
            // Best-effort: silently skip metric if request details are unavailable.
            Ok(None) | Err(_) => return,
        };

        if let Some(fulfilled_at) = proof_request.fulfilled_at {
            let fulfillment_duration_secs = fulfilled_at - proof_request.created_at;
            sov_metrics::track_metrics(|tracker| {
                tracker.submit(metrics::SP1ProofFulfillmentMetrics {
                    request_id: format!("{request_id}"),
                    fulfillment_duration_secs,
                });
            });
        }
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
            Some(proof) => {
                #[cfg(feature = "metrics")]
                self.emit_fulfillment_metric(*handle).await;

                Ok(Some(bincode::serialize(&proof)?))
            }
            None => Ok(None),
        }
    }

    fn code_commitment(
        &self,
    ) -> anyhow::Result<<<Self::Guest as ZkvmGuest>::Verifier as ZkVerifier>::CodeCommitment> {
        Ok(crate::SP1MethodId(self.pk.verifying_key().hash_u32()))
    }
}
