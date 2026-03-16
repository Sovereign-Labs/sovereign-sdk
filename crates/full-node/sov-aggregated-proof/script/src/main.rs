use std::borrow::Borrow;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, ensure, Context};
use slop_algebra::PrimeField32;
use sov_aggregated_proof_shared::DeferredProofInput;
use sov_mock_da::MockDaSpec;
use sov_sp1_adapter::BlockHeaderWithProof;
use sp1_recursion_executor::RecursionPublicValues;
use sp1_sdk::blocking::{ProveRequest, Prover, ProverClient};
use sp1_sdk::prelude::{include_elf, Elf, HashableKey, SP1Stdin};
use sp1_sdk::{ProvingKey, SP1Proof, SP1VerifyingKey};

const AGGREGATION_ELF: Elf = include_elf!("sov-aggregated-proof-program");

fn main() -> anyhow::Result<()> {
    let start = Instant::now();
    let (raw_proofs, verification_key) = proofs_and_vk();
    ensure!(
        !raw_proofs.is_empty(),
        "At least one proof file is required"
    );

    println!(
        "[host] starting aggregated proof check for {} proof file(s)",
        raw_proofs.len()
    );

    let prover = ProverClient::builder().cpu().build();

    let aggregation_pk = prover
        .setup(AGGREGATION_ELF)
        .context("Failed to set up the outer SP1 aggregation program")?;

    let inner_vk_hash = verification_key.hash_u32();

    let mut stdin = SP1Stdin::new();
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

        let deferred_proof_input = DeferredProofInput {
            public_values: proof.public_values.to_vec(),
            vkey_hash: inner_vk_hash,
            da_block_header: block_header_with_proof.da_block_header,
        };

        proof_inputs.push(deferred_proof_input);
        stdin.write_proof(*recursion_proof.clone(), verification_key.vk.clone());
    }

    stdin.write(&proof_inputs);

    println!("[host] starting outer compressed proof");
    let outer_proof = prover
        .prove(&aggregation_pk, stdin)
        .compressed()
        .run()
        .context("Failed to prove the outer SP1 aggregation program")?;

    prover
        .verify(&outer_proof, aggregation_pk.verifying_key(), None)
        .context("Failed to verify the outer SP1 aggregation proof")?;

    println!("[host] outer proof verified in {:?}", start.elapsed());

    Ok(())
}

fn proofs_and_vk() -> (Vec<BlockHeaderWithProof<MockDaSpec>>, SP1VerifyingKey) {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_dir = manifest_dir
        .parent()
        .expect("script crate must live under the sov-aggregated-proof workspace root");

    let data_dir = workspace_dir.join("data");

    let vk = read_saved_inner_vk(&data_dir.join("inner_vk.bin")).unwrap();

    let mut block_headers_with_proofs = Vec::new();

    let paths = [
        data_dir.join("inner_0_proof.json"),
        data_dir.join("inner_1_proof.json"),
        data_dir.join("inner_2_proof.json"),
    ];

    for path in paths {
        block_headers_with_proofs.push(read_saved_proof(&path).unwrap());
    }

    (block_headers_with_proofs, vk)
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
