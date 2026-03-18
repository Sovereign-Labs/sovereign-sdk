mod network;
mod parallel;

use sov_mock_da::{MockBlockHeader, MockDaSpec, MockHash};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::{DaProof, RelevantBlobs, RelevantProofs, Time};
use sov_rollup_interface::zk::StateTransitionWitness;
use sov_stf_runner::processes::{
    ProofAggregationStatus, ProverService, StateTransitionInfo,
};
use tokio::time;

use crate::helpers::RawGenesisStateRoot;

type StateRoot = Vec<u8>;
type Address = Vec<u8>;

fn make_transition_info(
    header_hash: MockHash,
    height: u64,
) -> StateTransitionInfo<StateRoot, Vec<u8>, MockDaSpec> {
    StateTransitionInfo::new(
        StateTransitionWitness {
            initial_state_root: Vec::default(),
            final_state_root: Vec::default(),
            da_block_header: MockBlockHeader {
                prev_hash: [0; 32].into(),
                hash: header_hash,
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
        SlotNumber::new_dangerous(height),
    )
}

async fn wait_for_aggregated_proof<P: ProverService<StateRoot = Vec<u8>, DaService = sov_mock_da::MockDaService>>(
    header_hashes: &[MockHash],
    genesis_state_root: &RawGenesisStateRoot,
    prover_service: &P,
) -> anyhow::Result<ProofAggregationStatus> {
    let mut counter = 0;
    loop {
        let status = prover_service
            .create_aggregated_proof(header_hashes, &genesis_state_root.0)
            .await?;

        if let ProofAggregationStatus::Success(_) = &status {
            return Ok(status);
        }

        if counter == 10 {
            return Ok(status);
        }

        time::sleep(time::Duration::from_millis(1000)).await;
        counter += 1;
    }
}
