use borsh::BorshSerialize;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use sov_rollup_interface::optimistic::{Attestation, BondingProofService, SerializedAttestation};
use sov_rollup_interface::stf::ProofSender;
use tokio::task::JoinHandle;

use crate::processes::{Receiver, StateTransitionInfo};

/// Manages the lifecycle of the [`Attestation`].
pub struct AttestationsManager<StateRoot, Witness, Da: DaSpec, Bps: BondingProofService> {
    stf_info_receiver: Receiver<StateRoot, Witness, Da>,
    bonding_proof_service: Bps,
    proof_sender: Box<dyn ProofSender>,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
}

impl<StateRoot, Witness, Da, Bps> AttestationsManager<StateRoot, Witness, Da, Bps>
where
    Bps: BondingProofService,
    Da: DaSpec,
    StateRoot: BorshSerialize + Serialize + DeserializeOwned + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    /// Creates a new [`AttestationsManager`]
    pub fn new(
        stf_info_receiver: Receiver<StateRoot, Witness, Da>,
        bonding_proof_service: Bps,
        proof_sender: Box<dyn ProofSender>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
    ) -> Self {
        Self {
            stf_info_receiver,
            bonding_proof_service,
            proof_sender,
            shutdown_receiver,
        }
    }

    /// Starts a background task for `Attestation` generation.
    pub async fn post_attestation_to_da_in_background(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            if let Err(e) = self.post_attestation_to_da().await {
                tracing::error!(error = ?e, "Failed to post attestation to DA");
            }
        })
    }

    async fn post_attestation_to_da(mut self) -> anyhow::Result<()> {
        loop {
            match future_or_shutdown(self.stf_info_receiver.read_next(), &self.shutdown_receiver)
                .await
            {
                FutureOrShutdownOutput::Shutdown => {
                    tracing::info!("Shutting down attestations posting task...");
                    break;
                }
                FutureOrShutdownOutput::Output(stf_info_result) => {
                    let stf_info = match stf_info_result? {
                        None => {
                            tracing::debug!("Received None instead of StateTransitionInfo. This can happen if the transition has already been processed by the `Receiver`. In that case, it is fine to ignore the notification.");
                            continue;
                        }
                        Some(stf_info) => stf_info,
                    };

                    self.process_stf_info(stf_info).await?;
                }
            }
        }
        tracing::debug!("Attestations posting task has been completed");
        Ok(())
    }

    async fn process_stf_info(
        &mut self,
        stf_info: StateTransitionInfo<StateRoot, Witness, Da>,
    ) -> anyhow::Result<()> {
        let slot_number = stf_info.slot_number;
        let witness = stf_info.witness();

        let attestation = Attestation {
            initial_state_root: witness.initial_state_root,
            slot_hash: witness.da_block_header.hash(),
            post_state_root: witness.final_state_root,
            proof_of_bond: self
                .bonding_proof_service
                .get_bonding_proof(slot_number)
                .ok_or(anyhow::anyhow!(
                    "Cannot get bonding proof, storage is corrupted."
                ))?,
        };

        let attestation_height = attestation.proof_of_bond.claimed_slot_number;
        let serialized_attestation = SerializedAttestation::from_attestation(&attestation)?;

        self.proof_sender
            .publish_attestation_blob_with_metadata(serialized_attestation)
            .await?;

        tracing::debug!(%slot_number, %attestation_height, "Submitting attestation to DA");

        self.stf_info_receiver.inc_next_height_to_receive();
        Ok(())
    }
}
