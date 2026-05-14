mod parallel;

use sov_mock_da::{MockBlockHeader, MockDaSpec, MockHash};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::{DaProof, RelevantBlobs, RelevantProofs, Time};
use sov_rollup_interface::zk::StateTransitionWitness;
use sov_stf_runner::processes::{ProofAggregationStatus, ProverService, StateTransitionInfo};
use tokio::time;

type StateRoot = Vec<u8>;
type Address = Vec<u8>;

fn make_header(header_hash: MockHash, height: u64) -> MockBlockHeader {
    make_header_with_prev(header_hash, [0; 32].into(), height)
}

fn make_header_with_prev(
    header_hash: MockHash,
    prev_hash: MockHash,
    height: u64,
) -> MockBlockHeader {
    MockBlockHeader {
        prev_hash,
        hash: header_hash,
        height,
        time: Time::now(),
    }
}

/// Builds a vector of `count` headers where each header's `prev_hash` points
/// at the previous header's `hash`. Required when feeding multiple headers
/// into proof aggregation, which now enforces DA hash-chain continuity.
pub(crate) fn make_chained_headers(count: usize) -> Vec<MockBlockHeader> {
    (0..count)
        .map(|height| {
            let hash = MockHash::from([(height + 1) as u8; 32]);
            let prev_hash = MockHash::from([(height) as u8; 32]);
            make_header_with_prev(hash, prev_hash, height as u64)
        })
        .collect()
}

fn make_transition_info(
    da_block_header: MockBlockHeader,
) -> StateTransitionInfo<StateRoot, Vec<u8>, MockDaSpec> {
    let slot_number = SlotNumber::new_dangerous(da_block_header.height + 1);
    StateTransitionInfo::new(StateTransitionWitness {
        initial_state_root: Vec::default(),
        final_state_root: Vec::default(),
        da_block_header,
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
        slot_number,
    })
}

async fn wait_for_aggregated_proof<
    P: ProverService<StateRoot = Vec<u8>, DaService = sov_mock_da::MockDaService>,
>(
    block_headers: &[MockBlockHeader],
    prover_service: &P,
) -> anyhow::Result<ProofAggregationStatus> {
    let mut counter = 0;
    loop {
        let status = prover_service
            .create_aggregated_proof(block_headers)
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
