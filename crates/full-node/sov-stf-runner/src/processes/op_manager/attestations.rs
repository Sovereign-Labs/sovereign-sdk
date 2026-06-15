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
        let slot_number = stf_info.slot_number();
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

        self.stf_info_receiver
            .inc_next_height_to_receive_by_and_persist(1)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use async_trait::async_trait;
    use sov_db::proof_manager_db::ProofManagerDb;
    use sov_mock_da::{MockBlockHeader, MockDaSpec, MockHash};
    use sov_modules_api::da::Time;
    use sov_rollup_interface::common::SlotNumber;
    use sov_rollup_interface::da::{DaProof, RelevantBlobs, RelevantProofs};
    use sov_rollup_interface::optimistic::{ProofOfBond, SerializedChallenge};
    use sov_rollup_interface::stf::ProofSender;
    use sov_rollup_interface::zk::StateTransitionWitness;
    use tokio::sync::watch;

    use super::*;
    use crate::processes::{new_stf_info_channel, StateTransitionInfo};

    struct TestBondingProofService;

    impl BondingProofService for TestBondingProofService {
        type StateProof = ();

        fn get_bonding_proof(
            &self,
            slot_number: SlotNumber,
        ) -> Option<ProofOfBond<Self::StateProof>> {
            Some(ProofOfBond {
                claimed_slot_number: slot_number,
                proof: (),
            })
        }
    }

    struct NoopProofSender;

    #[async_trait]
    impl ProofSender for NoopProofSender {
        async fn publish_proof_blob_with_metadata(
            &self,
            _serialized_proof: sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn publish_attestation_blob_with_metadata(
            &self,
            _serialized_attestation: SerializedAttestation,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn publish_challenge_blob_with_metadata(
            &self,
            _serialized_challenge: SerializedChallenge,
            _slot_height: SlotNumber,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_attestation_processing_persists_next_height_to_receive() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let proof_manager_db = ProofManagerDb::open(temp_dir.path())?;

        let (_sender, receiver) = new_stf_info_channel::<Vec<u8>, Vec<u8>, MockDaSpec>(
            proof_manager_db.clone(),
            NonZero::new(4).unwrap(),
            NonZero::new(4).unwrap(),
        )?;

        let (_shutdown_sender, shutdown_receiver) = watch::channel(());
        let mut manager = AttestationsManager::new(
            receiver,
            TestBondingProofService,
            Box::new(NoopProofSender),
            shutdown_receiver,
        );

        manager.process_stf_info(make_stf_info(1)).await?;

        assert_eq!(
            proof_manager_db.get_next_height_to_receive()?,
            Some(SlotNumber::new(2))
        );
        Ok(())
    }

    fn make_stf_info(height: u64) -> StateTransitionInfo<Vec<u8>, Vec<u8>, MockDaSpec> {
        StateTransitionInfo::new(
            StateTransitionWitness {
                initial_state_root: vec![1, 2, 3],
                final_state_root: vec![3, 4, 5],
                da_block_header: MockBlockHeader {
                    prev_hash: [0; 32].into(),
                    hash: MockHash([height as u8; 32]),
                    height,
                    time: Time::now(),
                },
                relevant_proofs: RelevantProofs {
                    batch: DaProof {
                        inclusion_proof: Default::default(),
                        completeness_proof: Default::default(),
                    },
                    proof: DaProof {
                        inclusion_proof: Default::default(),
                        completeness_proof: Default::default(),
                    },
                },
                relevant_blobs: RelevantBlobs {
                    proof_blobs: vec![],
                    batch_blobs: vec![],
                },
                witness: vec![],
            },
            SlotNumber::new(height),
        )
    }
}
