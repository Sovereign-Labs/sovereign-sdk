use std::path::PathBuf;

use super::{DefaultSpec, ProofStateRoot, ProofWitness, StfWitness};
use sov_mock_da::MockDaSpec;
use sov_modules_api::{Spec, ZkVerifier};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::zk::aggregated_proof::BlockHeaderWithProof;
use sov_rollup_interface::zk::SerializedZkProof;
use sov_rollup_interface::zk::{StateTransitionPublicData, StateTransitionWitnessWithAddress};
use sov_sp1_adapter::host::SP1Prover;
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

    let v_key = host.prover.verifying_key();
    let method_id = host.prover.method_id();

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

/// This test reproduces the proof generation process for the rollup used in benchmarks. It also
/// enforces the [`assert_native_matches_zk`] invariant for bank token creation + transfers.
#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(skip_guest_build, ignore)]
async fn test_proof_generation() {
    let (_genesis_state_root, witnesses) = super::generate_witnesses().await;
    assert_native_matches_zk(witnesses).await;
}

/// Enforces the [`assert_native_matches_zk`] invariant for a block that deploys and exercises an EVM
/// contract (the path most likely to grow native-only execution shortcuts).
#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(skip_guest_build, ignore)]
async fn test_evm_deploy_and_call_proof_generation() {
    let (_genesis_state_root, witnesses) = super::generate_evm_witnesses().await;
    assert_eq!(witnesses.len(), 1);
    assert_native_matches_zk(witnesses).await;
}

/// SDK consistency invariant: **the ZK guest must reproduce native execution exactly.**
///
/// For each block's witness, this runs the guest — which re-executes the STF *cache-free*
/// ([`ExecutionContext::Zk`], with no `#[cfg(feature = "native")]` code) — and asserts the proven
/// initial/final state roots equal the natively-computed ones.
///
/// Crucially, the final state root commits the *entire* provable state, including the per-block
/// `gas_used` recorded by `sov-chain-state`. So a native-only divergence in gas accounting (e.g. a
/// native cache or optimization the guest does not share) shows up here as a final-root mismatch —
/// this is the general guard whose absence let such a divergence ship undetected. Any future
/// native-only execution shortcut MUST keep this invariant passing.
async fn assert_native_matches_zk(witnesses: Vec<StfWitness>) {
    // Mock prover: still *executes* the guest (the cache-free re-execution we rely on), but skips the
    // slow STARK so this can run in tests.
    std::env::set_var("SP1_PROVER", "mock");

    assert!(
        !witnesses.is_empty(),
        "expected at least one block witness to verify"
    );

    let host = TestHost::new().await;
    let method_id = host.prover.method_id();
    let prover_address = default_prover_address();

    for witness in witnesses {
        let initial_state_root = witness.initial_state_root;
        let final_state_root = witness.final_state_root;
        let slot_number = witness.slot_number;

        // The guest re-executes the state transition from the witness. If native produced a
        // transition the cache-free guest cannot reproduce (e.g. an incomplete witness from a
        // native-only read path), this step itself fails.
        let proof = generate_proof(&host, witness, prover_address).await;
        let proof_public_data = verify(&proof.proof, method_id.clone()).await;

        assert_eq!(proof_public_data.slot_hash, proof.da_block_header.hash());
        assert_eq!(
            proof_public_data.initial_state_root, initial_state_root,
            "slot {slot_number:?}: ZK-guest initial state root must equal native execution"
        );
        assert_eq!(
            proof_public_data.final_state_root, final_state_root,
            "slot {slot_number:?}: ZK-guest final state root must equal native execution. The final \
             root commits the provable `gas_used`, so this also guards gas calculation against \
             native-only divergence."
        );
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
    prover: SP1Prover,
}

impl TestHost {
    async fn new() -> Self {
        let prover = tokio::task::spawn_blocking(move || {
            SP1Prover::new(*sp1::SP1_GUEST_MOCK_ELF)
                .expect("SP1Prover should be created successfully")
        })
        .await
        .unwrap();

        Self { prover }
    }

    async fn run(&self, data: ProofInput) -> SerializedZkProof {
        let prover = self.prover.clone();
        tokio::task::spawn_blocking(move || -> SerializedZkProof {
            prover
                .add_hint_and_run(&data)
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
