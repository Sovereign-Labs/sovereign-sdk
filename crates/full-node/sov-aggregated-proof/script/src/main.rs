use std::borrow::Borrow;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, ensure, Context};
use serde::Deserialize;
use slop_algebra::PrimeField32;
use sov_aggregated_proof_shared::DeferredProofInput;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::Spec;
use sov_rollup_interface::execution_mode::Zk;
use sov_rollup_interface::zk::{Proof, StateTransitionPublicData};
use sov_sp1_adapter::SP1;
use sov_state::Storage;
use sp1_recursion_executor::RecursionPublicValues;
use sp1_sdk::blocking::{ProveRequest, Prover, ProverClient};
use sp1_sdk::prelude::{include_elf, Elf, HashableKey, SP1Stdin};
use sp1_sdk::{ProvingKey, SP1Proof, SP1ProofWithPublicValues, SP1PublicValues, SP1VerifyingKey};

type S = DefaultSpec<MockDaSpec, SP1, MockZkvm, Zk>;

const AGGREGATION_ELF: Elf = include_elf!("sov-aggregated-proof-program");

#[derive(Deserialize)]
struct SavedProof {
    proof: Vec<u8>,
}

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

    for (index, proof) in raw_proofs.into_iter().enumerate() {
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

fn proofs_and_vk() -> (Vec<SP1ProofWithPublicValues>, SP1VerifyingKey) {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_dir = manifest_dir
        .parent()
        .expect("script crate must live under the sov-aggregated-proof workspace root");

    let data_dir = workspace_dir.join("data");

    let vk = read_saved_inner_vk(&data_dir.join("inner_vk.bin")).unwrap();

    let mut proofs = Vec::new();

    let paths = [
        data_dir.join("inner_0_proof.json"),
        data_dir.join("inner_1_proof.json"),
        data_dir.join("inner_2_proof.json"),
    ];

    for path in paths {
        proofs.push(read_saved_proof(&path).unwrap());
    }

    (proofs, vk)
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

fn read_saved_proof(file_path: &Path) -> anyhow::Result<SP1ProofWithPublicValues> {
    let file_contents = fs::read(file_path).with_context(|| {
        format!(
            "Failed to read saved proof fixture at {}",
            file_path.display()
        )
    })?;
    let saved_proof: SavedProof = serde_json::from_slice(&file_contents).with_context(|| {
        format!(
            "Failed to deserialize saved proof JSON at {}",
            file_path.display()
        )
    })?;

    let proof: Proof<SP1ProofWithPublicValues, SP1PublicValues> =
        bincode::deserialize(&saved_proof.proof).with_context(|| {
            format!(
                "Failed to deserialize saved SP1 proof bytes from {}",
                file_path.display()
            )
        })?;

    match proof {
        Proof::Full(full_proof) => {
            let _: StateTransitionPublicData<
                <S as Spec>::Address,
                MockDaSpec,
                <<S as Spec>::Storage as Storage>::Root,
            > = bincode::deserialize(full_proof.public_values.as_slice()).with_context(|| {
                format!(
                    "Failed to deserialize StateTransitionPublicData from the public values in {}",
                    file_path.display()
                )
            })?;

            Ok(full_proof)
        }
        Proof::PublicData(_) => {
            bail!(
                "Saved proof fixture {} does not contain a full proof",
                file_path.display()
            )
        }
    }
}
