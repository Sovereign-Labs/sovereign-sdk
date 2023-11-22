use sov_mock_da::{MockBlockHeader, MockDaService, MockDaVerifier, MockValidityCond};
use sov_mock_zkvm::MockZkvm;
use sov_stf_runner::{
    mock::MockStf, ParallelProverService, ProverService, RollupProverConfig, StateTransitionData,
};

#[tokio::test]
async fn test_prover_prove() {
    let vm = MockZkvm {};
    let prover_config = RollupProverConfig::Execute;
    let zk_stf = MockStf::<MockValidityCond>::default();
    let da_verifier = MockDaVerifier::default();

    let prover_service: ParallelProverService<[u8; 32], Vec<u8>, MockDaService, _, _> =
        ParallelProverService::new(vm, zk_stf, da_verifier, prover_config, ());

    let state_transition_data = StateTransitionData {
        pre_state_root: [0; 32],
        da_block_header: MockBlockHeader {
            prev_hash: [0; 32].into(),
            hash: [0; 32].into(),
            height: 0,
        },
        inclusion_proof: [0; 32],
        completeness_proof: (),
        blobs: vec![],
        state_transition_witness: vec![],
    };
    prover_service.submit_witness(state_transition_data).await;
    prover_service.prove([0; 32]).await.unwrap();
}
