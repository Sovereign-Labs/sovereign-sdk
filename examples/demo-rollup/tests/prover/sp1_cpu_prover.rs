use std::path::PathBuf;

use super::{DefaultSpec, ProofStateRoot, ProofWitness};
use sov_mock_da::MockDaSpec;
use sov_modules_api::{Spec, ZkVerifier};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::zk::aggregated_proof::BlockHeaderWithProof;
use sov_rollup_interface::zk::SerializedInnerProof;
use sov_rollup_interface::zk::{
    StateTransitionPublicData, StateTransitionWitnessWithAddress, ZkvmHost,
};
use sov_sp1_adapter::host::{MockSp1Prover, SP1Host};
use sov_sp1_adapter::{SP1MethodId, SP1Verifier};
use tokio::time::Instant;

type ProofInput = StateTransitionWitnessWithAddress<
    <DefaultSpec as Spec>::Address,
    ProofStateRoot,
    ProofWitness,
    MockDaSpec,
>;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "This test is used to generate data for testing the aggregate proof circuit and should be enabled only when needed."]
async fn test_save_proofs() {
    let (host, code_commitment) = TestHost::new(true).await;
    let proof_data = generate_proofs(&host).await;
    let proofs_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("test_data")
        .join("tmp");

    std::fs::create_dir_all(&proofs_dir).unwrap();
    std::fs::write(proofs_dir.join("inner_vk.bin"), code_commitment.0.clone()).unwrap();

    for (i, data) in proof_data.into_iter().enumerate() {
        let proof_public_data =
            verify(data.proof.raw_inner_proof.clone(), code_commitment.clone()).await;
        assert_eq!(proof_public_data.slot_hash, data.da_block_header.hash());
        let json = serde_json::to_string(&data).unwrap();
        std::fs::write(proofs_dir.join(format!("inner_{i}_proof.json")), &json).unwrap();
    }
}

/// This test reproduces the proof generation process for the rollup used in benchmarks.
#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(skip_guest_build, ignore)]
async fn test_proof_generation() {
    let (host, _) = TestHost::new(false).await;
    let _ = generate_proofs(&host).await;
}

async fn generate_proofs(host: &TestHost) -> Vec<BlockHeaderWithProof<MockDaSpec>> {
    let (_genesis_state_root, witnesses) = super::generate_witnesses().await;

    let prover_address = <DefaultSpec as Spec>::Address::try_from([0u8; 28].as_ref()).unwrap();
    let mut proofs = Vec::new();

    for (i, witness) in witnesses.into_iter().enumerate() {
        let da_block_header = witness.da_block_header.clone();

        let data: ProofInput = StateTransitionWitnessWithAddress {
            stf_witness: witness,
            prover_address,
        };

        let start = Instant::now();
        let raw_inner_proof = host.run(data).await;
        println!("XX {i} proof generated in {:?}", start.elapsed());
        proofs.push(BlockHeaderWithProof {
            da_block_header,
            proof: SerializedInnerProof { raw_inner_proof },
        });
    }

    proofs
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mock_proof_public_data_matches_witnesses() {
    let (host, _) = TestHost::new(false).await;
    let (_genesis_state_root, witnesses) = super::generate_witnesses().await;
    let prover_address = <DefaultSpec as Spec>::Address::try_from([0u8; 28].as_ref()).unwrap();

    for witness in witnesses {
        let expected_initial_state_root = witness.initial_state_root.clone();
        let expected_final_state_root = witness.final_state_root.clone();
        let expected_slot_hash = witness.da_block_header.hash();

        let public_data = host
            .run_mock_public_data(ProofInput {
                stf_witness: witness,
                prover_address,
            })
            .await;

        assert_eq!(public_data.initial_state_root, expected_initial_state_root);
        assert_eq!(public_data.final_state_root, expected_final_state_root);
        assert_eq!(public_data.slot_hash, expected_slot_hash);
    }
}

// The SP1 prover manages its own Tokio runtime, which conflicts with the `tokio::test` runtime.
// To avoid this, all blocking work must be executed inside `tokio::task::spawn_blocking`.
struct TestHost {
    host: SP1Host,
    mock_host: MockSp1Prover,
    with_proof: bool,
}

impl TestHost {
    async fn new(with_proof: bool) -> (Self, SP1MethodId) {
        let (code_commitment, host, mock_host) = tokio::task::spawn_blocking(move || {
            let host = SP1Host::new(*sp1::SP1_GUEST_MOCK_ELF)
                .expect("SP1Host should be created successfully");

            let mock_host = MockSp1Prover::new(*sp1::SP1_GUEST_MOCK_ELF)
                .expect("MockSp1Prover should be created successfully");

            (
                host.code_commitment()
                    .expect("SP1 code commitment should be created successfully"),
                host,
                mock_host,
            )
        })
        .await
        .unwrap();

        (
            Self {
                host,
                mock_host,
                with_proof,
            },
            code_commitment,
        )
    }

    async fn run(&self, data: ProofInput) -> Vec<u8> {
        if self.with_proof {
            let mut host = self.host.clone();
            tokio::task::spawn_blocking(move || -> Vec<u8> {
                host.add_hint_and_run(&data)
                    .expect("Prover should run successfully")
            })
            .await
            .unwrap()
        } else {
            let mock_host = self.mock_host.clone();
            tokio::task::spawn_blocking(move || -> Vec<u8> {
                mock_host.add_hint_and_run(&data).unwrap();
                Default::default()
            })
            .await
            .unwrap()
        }
    }

    async fn run_mock_public_data(
        &self,
        data: ProofInput,
    ) -> StateTransitionPublicData<<DefaultSpec as Spec>::Address, MockDaSpec, ProofStateRoot> {
        tokio::task::spawn_blocking(move || {
            let mock_host = MockSp1Prover::new(*sp1::SP1_GUEST_MOCK_ELF)
                .expect("MockSp1Prover should be created successfully");

            let proof = mock_host
                .add_hint_and_run(&data)
                .expect("Mock prover should run successfully");

            bincode::deserialize(proof.public_values.as_slice())
                .expect("Mock proof public values should deserialize")
        })
        .await
        .unwrap()
    }
}

#[allow(dead_code)]
async fn verify(
    proof: Vec<u8>,
    code_commitment: SP1MethodId,
) -> StateTransitionPublicData<<DefaultSpec as Spec>::Address, MockDaSpec, ProofStateRoot> {
    tokio::task::spawn_blocking(move || -> StateTransitionPublicData<<DefaultSpec as Spec>::Address, MockDaSpec, ProofStateRoot> {
            SP1Verifier::verify(&proof, &code_commitment)
                .expect("SP1 proof verification should succeed")
        })
        .await
        .unwrap()
}
