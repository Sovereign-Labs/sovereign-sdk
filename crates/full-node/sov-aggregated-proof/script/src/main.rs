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
use sov_modules_api::execution_mode::Zk;
use sov_modules_api::{
    AggregatedProofPublicData, CodeCommitmentHash, Spec, StateTransitionPublicData, Storage,
};
use sov_rollup_interface::zk::aggregated_proof::common::{
    AggregatedProofWitness, DeferredProofInput, PreviousOuterProofWitness,
};
use sov_sp1_adapter::BlockHeaderWithProof;
use sov_sp1_adapter::SP1;
use sp1_recursion_executor::RecursionPublicValues;
use sp1_sdk::blocking::{ProveRequest, Prover, ProverClient};
use sp1_sdk::prelude::{include_elf, Elf, HashableKey, SP1Stdin};
use sp1_sdk::{ProvingKey, SP1Proof, SP1ProofWithPublicValues, SP1VerifyingKey};

const AGGREGATION_ELF: Elf = include_elf!("sov-aggregated-proof-program");
const JUMP: usize = 3;

type S = ConfigurableSpec<MockDaSpec, SP1, MockZkvm, MultiAddressEvmSolana, Zk>;

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

    let verification_key = saved_inner_vk()?;
    let prover = ProverClient::builder().cpu().build();

    let aggregation_pk = prover.setup(AGGREGATION_ELF).map_err(|error| {
        anyhow::anyhow!("Failed to set up the outer SP1 aggregation program: {error}")
    })?;

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
                deserialize_pub_data::<AggregatedProofPublicData<<S as Spec>::Address, MockDaSpec, <<S as Spec>::Storage as Storage>::Root>>(proof.public_values.as_slice())
            })
            .transpose()
            .context("Failed to deserialize previous outer proof public data")?;

        let (expected_initial_state_root, expected_final_state_root) =
            batch_state_roots(&proof_batch)
                .context("Failed to derive expected state roots from the current proof batch")?;

        let outer_proof = create_agg_proof(
            &prover,
            &aggregation_pk,
            &verification_key,
            proof_batch,
            previous_outer_proof.take(),
        )?;

        let public_data: AggregatedProofPublicData<<S as Spec>::Address, MockDaSpec, <<S as Spec>::Storage as Storage>::Root> =
            deserialize_pub_data(outer_proof.public_values.as_slice())
                .context("Failed to deserialize outer proof public data")?;

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

        previous_outer_proof = Some(outer_proof);
    }

    println!("[host] verified outer proof(s) in {:?}", start.elapsed());

    Ok(())
}

fn create_agg_proof<P: Prover>(
    prover: &P,
    aggregation_pk: &P::ProvingKey,
    verification_key: &SP1VerifyingKey,
    raw_proofs: Vec<BlockHeaderWithProof<MockDaSpec>>,
    previous_outer_proof: Option<SP1ProofWithPublicValues>,
) -> anyhow::Result<SP1ProofWithPublicValues> {
    ensure!(
        !raw_proofs.is_empty(),
        "At least one proof file is required"
    );

    let aggregation_vk_hash = aggregation_pk.verifying_key().hash_u32();
    let inner_vk_hash = verification_key.hash_u32();
    let mut stdin = SP1Stdin::new();

    let prev_outer_proof_witness = if let Some(previous_outer_proof) = previous_outer_proof {
        let SP1Proof::Compressed(recursion_proof) = &previous_outer_proof.proof else {
            bail!("Expected the previous outer proof to be a compressed SP1 proof");
        };

        stdin.write_proof(
            *recursion_proof.clone(),
            aggregation_pk.verifying_key().vk.clone(),
        );

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
        stdin.write_proof(*recursion_proof.clone(), verification_key.vk.clone());
    }

    let outer_vkey_hash = CodeCommitmentHash::from_u32_array(aggregation_vk_hash);

    let witness = AggregatedProofWitness {
        proof_inputs,
        outer_vkey_hash: outer_vkey_hash,
        prev_outer_proof_witness,
    };

    stdin.write(&witness);

    let outer_proof = prover
        .prove(&aggregation_pk, stdin)
        .compressed()
        .run()
        .map_err(|error| {
            anyhow::anyhow!("Failed to prove the outer SP1 aggregation program: {error}")
        })?;

    prover
        .verify(&outer_proof, aggregation_pk.verifying_key(), None)
        .context("Failed to verify the outer SP1 aggregation proof")?;

    let public_data: AggregatedProofPublicData<<S as Spec>::Address, MockDaSpec, <<S as Spec>::Storage as Storage>::Root> =
        deserialize_pub_data(outer_proof.public_values.as_slice())
            .context("Failed to deserialize outer proof public data")?;

    println!(
        "[host] outer proof emits slots {}..={} with hashes {} -> {}",
        public_data.initial_slot_number,
        public_data.final_slot_number,
        public_data.initial_slot_hash,
        public_data.final_slot_hash
    );

    Ok(outer_proof)
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

fn saved_inner_vk() -> anyhow::Result<SP1VerifyingKey> {
    read_saved_inner_vk(&data_dir().join("inner_vk.bin"))
}

fn data_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_dir = manifest_dir
        .parent()
        .expect("script crate must live under the sov-aggregated-proof workspace root");

    workspace_dir.join("data")
}

fn read_saved_inner_vk(file_path: &Path) -> anyhow::Result<SP1VerifyingKey> {
    let file_contents = fs::read(file_path).with_context(|| {
        format!(
            "Failed to read saved verifying key fixture at {}",
            file_path.display()
        )
    })?;

    bincode::deserialize(&file_contents).with_context(|| {
        format!(
            "Failed to deserialize saved SP1 verifying key bytes from {}",
            file_path.display()
        )
    })
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

    let first_public_data: StateTransitionPublicData<<S as Spec>::Address, MockDaSpec, <<S as Spec>::Storage as Storage>::Root> = deserialize_pub_data(
        sov_sp1_adapter::decode_sp1_proof(&first_proof.proof)?
            .public_values
            .as_slice(),
    )?;
    let last_public_data: StateTransitionPublicData<<S as Spec>::Address, MockDaSpec, <<S as Spec>::Storage as Storage>::Root> = deserialize_pub_data(
        sov_sp1_adapter::decode_sp1_proof(&last_proof.proof)?
            .public_values
            .as_slice(),
    )?;

    Ok((
        first_public_data.initial_state_root,
        last_public_data.final_state_root,
    ))
}
