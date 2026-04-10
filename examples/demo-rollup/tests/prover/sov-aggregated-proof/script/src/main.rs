use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{ensure, Context};
use demo_stf::MultiAddressEvmSolana;

use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::{AggregatedProofPublicData, Spec, StateTransitionPublicData, Storage};
use sov_rollup_interface::execution_mode::Native;
use sov_rollup_interface::zk::aggregated_proof::BlockHeaderWithProof;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_rollup_interface::zk::ZkVerifier;
use sov_sp1_adapter::host::SP1AggregationHost;
use sov_sp1_adapter::SP1;
use sov_sp1_adapter::{SP1MethodId, SP1Verifier};
use sp1_sdk::prelude::{include_elf, Elf};

const AGGREGATION_ELF: Elf = include_elf!("sov-aggregated-proof-program");
const JUMP: usize = 3;

type S = ConfigurableSpec<MockDaSpec, SP1, MockZkvm, MultiAddressEvmSolana, Native>;

fn main() -> anyhow::Result<()> {
    let start = Instant::now();
    let raw_proofs = proofs();
    ensure!(
        !raw_proofs.is_empty(),
        "At least one proof file is required"
    );
    ensure!(
        raw_proofs.len() % JUMP == 0,
        "Expected the number of inner proofs ({}) to be divisible by jump ({JUMP})",
        raw_proofs.len()
    );

    println!(
        "[host] starting aggregated proof check for {} proof file(s)",
        raw_proofs.len()
    );

    let verification_key = SP1MethodId(saved_inner_vk_bytes()?);

    let mut prover = SP1AggregationHost::new(&AGGREGATION_ELF, verification_key)?;

    let proof_batches = raw_proofs
        .chunks(JUMP)
        .map(|proof_batch| proof_batch.to_vec())
        .collect::<Vec<_>>();

    let num_outer_proofs = proof_batches.len();
    let code_commitment = prover.code_commitment();

    for (batch_index, proof_batch) in proof_batches.into_iter().enumerate() {
        println!(
            "[host] creating outer proof {}/{} from {} inner proof(s)",
            batch_index + 1,
            num_outer_proofs,
            proof_batch.len()
        );

        let (expected_initial_state_root, expected_final_state_root) =
            batch_state_roots(&proof_batch)
                .context("Failed to derive expected state roots from the current proof batch")?;

        let outer_proof_bytes = SerializedAggregatedProof {
            raw_aggregated_proof: prover.run::<MockDaSpec>(proof_batch)?,
        };

        let public_data: AggregatedProofPublicData<
            <S as Spec>::Address,
            MockDaSpec,
            <<S as Spec>::Storage as Storage>::Root,
        > = SP1Verifier::verify(&outer_proof_bytes.raw_aggregated_proof, &code_commitment)
            .context("Failed to verify the outer SP1 aggregation proof")?;

        println!(
            "[host] outer proof emits slots {}..={} with hashes {} -> {}",
            public_data.initial_slot_number,
            public_data.final_slot_number,
            public_data.initial_slot_hash,
            public_data.final_slot_hash
        );

        ensure!(
            public_data.initial_state_root == expected_initial_state_root,
            "Outer proof {} initial_state_root does not match the first inner proof initial_state_root",
            batch_index + 1
        );
        ensure!(
            public_data.final_state_root == expected_final_state_root,
            "Outer proof {} final_state_root does not match the last inner proof final_state_root",
            batch_index + 1
        );
    }

    println!("[host] verified outer proof(s) in {:?}", start.elapsed());

    Ok(())
}

fn proofs() -> Vec<BlockHeaderWithProof<MockDaSpec>> {
    let data_dir = data_dir();
    let mut block_headers_with_proofs = Vec::new();

    for i in 0..9 {
        let path = data_dir.join(format!("inner_{i}_proof.json"));
        block_headers_with_proofs.push(read_saved_proof(&path).unwrap());
    }

    block_headers_with_proofs
}

fn saved_inner_vk_bytes() -> anyhow::Result<Vec<u8>> {
    let path = data_dir().join("inner_vk.bin");
    fs::read(&path).with_context(|| {
        format!(
            "Failed to read saved verifying key fixture at {}",
            path.display()
        )
    })
}

fn data_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_dir = manifest_dir
        .parent()
        .expect("script crate must live under the sov-aggregated-proof workspace root");

    workspace_dir.join("data")
}

fn read_saved_proof(file_path: &Path) -> anyhow::Result<BlockHeaderWithProof<MockDaSpec>> {
    let file_contents = fs::read(file_path).with_context(|| {
        format!(
            "Failed to read saved proof fixture at {}",
            file_path.display()
        )
    })?;

    let block_header_with_proof: BlockHeaderWithProof<MockDaSpec> =
        serde_json::from_slice(&file_contents).with_context(|| {
            format!(
                "Failed to deserialize saved proof JSON at {}",
                file_path.display()
            )
        })?;

    Ok(block_header_with_proof)
}

fn deserialize_pub_data<T: serde::de::DeserializeOwned>(data: &[u8]) -> anyhow::Result<T> {
    bincode::deserialize(data).context("Failed to deserialize public data")
}

fn batch_state_roots(
    proof_batch: &[BlockHeaderWithProof<MockDaSpec>],
) -> anyhow::Result<(
    <<S as Spec>::Storage as Storage>::Root,
    <<S as Spec>::Storage as Storage>::Root,
)> {
    let first_proof = proof_batch
        .first()
        .expect("proof batches are guaranteed to be non-empty");
    let last_proof = proof_batch
        .last()
        .expect("proof batches are guaranteed to be non-empty");

    let first_public_data: StateTransitionPublicData<
        <S as Spec>::Address,
        MockDaSpec,
        <<S as Spec>::Storage as Storage>::Root,
    > = deserialize_pub_data(
        sov_sp1_adapter::decode_sp1_proof(&first_proof.proof.raw_inner_proof)?
            .public_values
            .as_slice(),
    )?;
    let last_public_data: StateTransitionPublicData<
        <S as Spec>::Address,
        MockDaSpec,
        <<S as Spec>::Storage as Storage>::Root,
    > = deserialize_pub_data(
        sov_sp1_adapter::decode_sp1_proof(&last_proof.proof.raw_inner_proof)?
            .public_values
            .as_slice(),
    )?;

    Ok((
        first_public_data.initial_state_root,
        last_public_data.final_state_root,
    ))
}
