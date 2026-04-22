use std::path::PathBuf;

use super::{DefaultSpec, ProofStateRoot, ProofWitness, StfWitness};
use sov_mock_da::MockDaSpec;
use sov_modules_api::{Spec, ZkVerifier};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::zk::aggregated_proof::BlockHeaderWithProof;
use sov_rollup_interface::zk::SerializedZkProof;
use sov_rollup_interface::zk::{
    StateTransitionPublicData, StateTransitionWitnessWithAddress, ZkvmHost,
};
use sov_sp1_adapter::host::SP1Host;
use sov_sp1_adapter::{SP1MethodId, SP1Verifier};

type ProofInput = StateTransitionWitnessWithAddress<
    <DefaultSpec as Spec>::Address,
    ProofStateRoot,
    ProofWitness,
    MockDaSpec,
>;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "This test is used to generate data for testing the aggregate proof circuit and should be enabled only when needed."]
async fn test_save_proofs() {
    let host = TestHost::new().await;
    let proof_data = generate_proofs(&host).await;
    let proofs_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("test_data")
        .join("tmp");

    let v_key = host.host.verifying_key();
    let method_id = host.host.method_id();

    std::fs::create_dir_all(&proofs_dir).unwrap();
    std::fs::write(
        proofs_dir.join("inner_vk.bin"),
        bincode::serialize(v_key).unwrap(),
    )
    .unwrap();

    for (i, data) in proof_data.into_iter().enumerate() {
        let proof_public_data = verify(&data.proof, method_id.clone()).await;
        assert_eq!(proof_public_data.slot_hash, data.da_block_header.hash());
        let json = serde_json::to_string(&data).unwrap();
        std::fs::write(proofs_dir.join(format!("inner_{i}_proof.json")), &json).unwrap();
    }
}

/// This test reproduces the proof generation process for the rollup used in benchmarks.
#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(skip_guest_build, ignore)]
async fn test_proof_generation() {
    // Use the mock prover: CPU proving is far too slow to run in tests.
    std::env::set_var("SP1_PROVER", "mock");

    let host = TestHost::new().await;
    let method_id = host.host.method_id();
    let (_genesis_state_root, witnesses) = super::generate_witnesses().await;
    let prover_address = default_prover_address();

    for witness in witnesses {
        let _initial_state_root = witness.initial_state_root;
        let _final_state_root = witness.final_state_root;

        let proof = generate_proof(&host, witness, prover_address).await;
        let proof_public_data = verify(&proof.proof, method_id.clone()).await;

        assert_eq!(proof_public_data.slot_hash, proof.da_block_header.hash());
        // TODO: Uncomment after NOmt bug is solved: https://github.com/Sovereign-Labs/sovereign-sdk/pull/2739
        //assert_eq!(proof_public_data.initial_state_root, initial_state_root);
        //assert_eq!(proof_public_data.final_state_root, final_state_root);
        assert_eq!(proof_public_data.prover_address, prover_address);
    }
}

async fn generate_proofs(host: &TestHost) -> Vec<BlockHeaderWithProof<MockDaSpec>> {
    let (_genesis_state_root, witnesses) = super::generate_witnesses().await;
    let prover_address = default_prover_address();

    let mut proofs = Vec::new();
    for witness in witnesses {
        proofs.push(generate_proof(host, witness, prover_address).await);
    }
    proofs
}

async fn generate_proof(
    host: &TestHost,
    witness: StfWitness,
    prover_address: <DefaultSpec as Spec>::Address,
) -> BlockHeaderWithProof<MockDaSpec> {
    let da_block_header = witness.da_block_header.clone();

    let data: ProofInput = StateTransitionWitnessWithAddress {
        stf_witness: witness,
        prover_address,
    };

    let inner_proof = host.run(data).await;
    BlockHeaderWithProof {
        da_block_header,
        proof: inner_proof,
    }
}

fn default_prover_address() -> <DefaultSpec as Spec>::Address {
    <DefaultSpec as Spec>::Address::try_from([0u8; 28].as_ref()).unwrap()
}

// The SP1 prover manages its own Tokio runtime, which conflicts with the `tokio::test` runtime.
// To avoid this, all blocking work must be executed inside `tokio::task::spawn_blocking`.
struct TestHost {
    host: SP1Host,
}

impl TestHost {
    async fn new() -> Self {
        let host = tokio::task::spawn_blocking(move || {
            SP1Host::new(*sp1::SP1_GUEST_MOCK_ELF).expect("SP1Host should be created successfully")
        })
        .await
        .unwrap();

        Self { host }
    }

    async fn run(&self, data: ProofInput) -> SerializedZkProof {
        let mut host = self.host.clone();
        tokio::task::spawn_blocking(move || -> SerializedZkProof {
            host.add_hint_and_run(&data)
                .expect("Prover should run successfully")
        })
        .await
        .unwrap()
    }
}

async fn verify(
    proof: &SerializedZkProof,
    code_commitment: SP1MethodId,
) -> StateTransitionPublicData<<DefaultSpec as Spec>::Address, MockDaSpec, ProofStateRoot> {
    SP1Verifier::verify_with_proof(proof, &code_commitment)
        .expect("SP1 proof verification should succeed")
}
