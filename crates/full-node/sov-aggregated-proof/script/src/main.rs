use std::borrow::Borrow;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, ensure, Context};
use demo_stf::MultiAddressEvmSolana;
use slop_algebra::PrimeField32;

use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::{
    AggregatedProofPublicData, CodeCommitmentHash, Spec, StateTransitionPublicData, Storage,
};
use sov_rollup_interface::execution_mode::Native;
use sov_rollup_interface::zk::aggregated_proof::common::{
    AggregatedProofWitness, DeferredProofInput, PreviousOuterProofWitness,
};
use sov_rollup_interface::zk::aggregated_proof::ZkvmHostWithInnerProofs;
use sov_rollup_interface::zk::{ZkVerifier, ZkvmHost};
use sov_sp1_adapter::host::SP1Host;
use sov_sp1_adapter::SP1;
use sov_sp1_adapter::{BlockHeaderWithProof, SP1MethodId, SP1Verifier};
use sp1_recursion_executor::RecursionPublicValues;
use sp1_sdk::prelude::{include_elf, Elf, HashableKey};
use sp1_sdk::{SP1Proof, SP1ProofWithPublicValues, SP1VerifyingKey};

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

    let mut prover = SP1Host::new(&AGGREGATION_ELF);

    let proof_batches = raw_proofs
        .chunks(JUMP)
        .map(|proof_batch| proof_batch.to_vec())
        .collect::<Vec<_>>();

    let num_outer_proofs = proof_batches.len();

    let mut previous_outer_proof: Option<SP1ProofWithPublicValues> = None;

    for (batch_index, proof_batch) in proof_batches.into_iter().enumerate() {
        println!(
            "[host] creating outer proof {}/{} from {} inner proof(s)",
            batch_index + 1,
            num_outer_proofs,
            proof_batch.len()
        );

        let previous_outer_public_data = previous_outer_proof
            .as_ref()
            .map(|proof| {
                deserialize_pub_data::<
                    AggregatedProofPublicData<
                        <S as Spec>::Address,
                        MockDaSpec,
                        <<S as Spec>::Storage as Storage>::Root,
                    >,
                >(proof.public_values.as_slice())
            })
            .transpose()
            .context("Failed to deserialize previous outer proof public data")?;

        let (expected_initial_state_root, expected_final_state_root) =
            batch_state_roots(&proof_batch)
                .context("Failed to derive expected state roots from the current proof batch")?;

        let (outer_proof_bytes, code_commitment) = create_agg_proof(
            &mut prover,
            &verification_key,
            proof_batch,
            previous_outer_proof.take(),
        )?;

        let public_data: AggregatedProofPublicData<
            <S as Spec>::Address,
            MockDaSpec,
            <<S as Spec>::Storage as Storage>::Root,
        > = SP1Verifier::verify(&outer_proof_bytes, &code_commitment)
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

        if let Some(previous_outer_public_data) = previous_outer_public_data.as_ref() {
            ensure!(
                public_data.genesis_state_root == previous_outer_public_data.genesis_state_root,
                "Outer proof {} genesis_state_root changed across recursive aggregation",
                batch_index + 1
            );
            ensure!(
                public_data.initial_state_root == previous_outer_public_data.final_state_root,
                "Outer proof {} initial_state_root does not continue the previous outer proof final_state_root",
                batch_index + 1
            );
        }

        let outer_proof = sov_sp1_adapter::decode_sp1_proof(&outer_proof_bytes)
            .context("Failed to decode outer proof")?;

        previous_outer_proof = Some(outer_proof);
    }

    println!("[host] verified outer proof(s) in {:?}", start.elapsed());

    Ok(())
}

fn create_agg_proof(
    agg_host: &mut SP1Host<'static>,
    verification_key: &SP1MethodId,
    raw_proofs: Vec<BlockHeaderWithProof<MockDaSpec>>,
    previous_outer_proof: Option<SP1ProofWithPublicValues>,
) -> anyhow::Result<(Vec<u8>, SP1MethodId)> {
    ensure!(
        !raw_proofs.is_empty(),
        "At least one proof file is required"
    );

    let aggregation_code_commitment = agg_host.code_commitment()?;
    let aggregation_vk: SP1VerifyingKey = bincode::deserialize(&aggregation_code_commitment.0)
        .context("Failed to deserialize aggregation SP1VerifyingKey")?;
    let aggregation_vk_hash = aggregation_vk.hash_u32();
    let inner_vk: SP1VerifyingKey = bincode::deserialize(&verification_key.0)
        .context("Failed to deserialize inner SP1VerifyingKey")?;
    let inner_vk_hash = inner_vk.hash_u32();

    let prev_outer_proof_witness = if let Some(previous_outer_proof) = previous_outer_proof {
        agg_host.add_proof(&previous_outer_proof, &aggregation_code_commitment)?;

        Some(PreviousOuterProofWitness {
            public_values: previous_outer_proof.public_values.to_vec(),
        })
    } else {
        None
    };

    let mut proof_inputs = Vec::with_capacity(raw_proofs.len());

    for (index, block_header_with_proof) in raw_proofs.into_iter().enumerate() {
        let proof = sov_sp1_adapter::decode_sp1_proof(&block_header_with_proof.proof)?;

        let SP1Proof::Compressed(recursion_proof) = &proof.proof else {
            bail!("Expected a compressed SP1 proof");
        };

        let public_values: &RecursionPublicValues<_> =
            recursion_proof.proof.public_values.as_slice().borrow();

        let expected_inner_vk_hash = public_values
            .sp1_vk_digest
            .map(|digest_word| digest_word.as_canonical_u32());

        ensure!(
            inner_vk_hash == expected_inner_vk_hash,
            "Saved verifying key does not match proof fixture {}",
            index
        );

        let deferred_proof_input = DeferredProofInput::<MockDaSpec> {
            public_values: proof.public_values.to_vec(),
            da_block_header: block_header_with_proof.da_block_header,
        };

        proof_inputs.push(deferred_proof_input);
        agg_host.add_proof(&proof, verification_key)?;
    }

    let outer_vkey_hash = CodeCommitmentHash::from_u32_array(aggregation_vk_hash);

    let witness = AggregatedProofWitness {
        proof_inputs,
        outer_vkey_hash,
        prev_outer_proof_witness,
    };

    agg_host.add_hint(witness);

    let outer_proof_bytes = agg_host.run(true)?;
    Ok((outer_proof_bytes, aggregation_code_commitment))
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
        sov_sp1_adapter::decode_sp1_proof(&first_proof.proof)?
            .public_values
            .as_slice(),
    )?;
    let last_public_data: StateTransitionPublicData<
        <S as Spec>::Address,
        MockDaSpec,
        <<S as Spec>::Storage as Storage>::Root,
    > = deserialize_pub_data(
        sov_sp1_adapter::decode_sp1_proof(&last_proof.proof)?
            .public_values
            .as_slice(),
    )?;

    Ok((
        first_public_data.initial_state_root,
        last_public_data.final_state_root,
    ))
}
