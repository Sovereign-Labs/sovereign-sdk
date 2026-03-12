use std::fs;

use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::{DaSpec, Spec};
use sov_rollup_interface::execution_mode::Zk;
use sov_rollup_interface::zk::{Proof, StateTransitionPublicData};
use sov_sp1_adapter::SP1;
use sov_state::Storage;
use sp1_sdk::blocking::{ProveRequest, Prover, ProverClient};
use sp1_sdk::{HashableKey, ProvingKey, SP1ProofWithPublicValues, SP1Stdin};

fn load_elf(path: &str) -> &'static [u8] {
    let elf = fs::read(path).unwrap_or_default();
    if elf.is_empty() {
        println!("Warning: ELF file at '{path}' is empty or could not be read");
    }
    Vec::leak(elf)
}

lazy_static! {
    pub static ref PROOF_AGGREGATION_ELF: &'static [u8] = load_elf(&format!(
        "{}/guest/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/proof-aggregation-sp1",
        env!("CARGO_MANIFEST_DIR")
    ));
}

type S = DefaultSpec<MockDaSpec, SP1, MockZkvm, Zk>;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BlockHeaderWithProof<Da: DaSpec> {
    da_block_header: Da::BlockHeader,
    proof: Vec<u8>,
}

fn read_proof(file_path: &str) -> anyhow::Result<BlockHeaderWithProof<MockDaSpec>> {
    let json = fs::read_to_string(file_path)?;
    Ok(serde_json::from_str(&json)?)
}

fn decode_sp1_proof(
    proof_bytes: &[u8],
) -> anyhow::Result<SP1ProofWithPublicValues> {
    let proof: Proof<SP1ProofWithPublicValues, sp1_sdk::blocking::SP1PublicValues> =
        bincode::deserialize(proof_bytes)?;
    match proof {
        Proof::Full(p) => Ok(p),
        Proof::PublicData(_) => anyhow::bail!("Expected full proof, got PublicData"),
    }
}

fn extract_witness(
    proof: &SP1ProofWithPublicValues,
) -> anyhow::Result<
    StateTransitionPublicData<
        <S as Spec>::Address,
        MockDaSpec,
        <<S as Spec>::Storage as Storage>::Root,
    >,
> {
    Ok(bincode::deserialize(proof.public_values.as_slice())?)
}

pub fn check_receipts(file_paths: Vec<&str>, inner_vkey_path: &str) -> anyhow::Result<()> {
    let prover = ProverClient::builder().cpu().build();
    let pk = prover.setup((*PROOF_AGGREGATION_ELF).into())?;

    // Load the inner program's verifying key.
    let inner_vkey_bytes = fs::read(inner_vkey_path)?;
    let inner_vkey: sp1_sdk::SP1VerifyingKey = bincode::deserialize(&inner_vkey_bytes)?;
    let vkey_hash = inner_vkey.hash_u64();

    let mut stdin = SP1Stdin::new();
    let mut witnesses = Vec::new();

    for file_path in &file_paths {
        let block_proof = read_proof(file_path)?;
        let sp1_proof = decode_sp1_proof(&block_proof.proof)?;
        let witness = extract_witness(&sp1_proof)?;
        witnesses.push(witness);

        // Extract the compressed recursion proof and write it for recursive verification.
        match sp1_proof.proof {
            sp1_sdk::SP1Proof::Compressed(recursion_proof) => {
                stdin.write_proof(*recursion_proof, inner_vkey.vk.clone());
            }
            other => anyhow::bail!(
                "Expected Compressed proof for aggregation, got: {}",
                other
            ),
        }
    }

    // Write the vkey hash and witnesses for the guest program.
    stdin.write(&vkey_hash);
    stdin.write(&witnesses);

    let receipt = prover
        .prove(&pk, stdin)
        .compressed()
        .run()
        .map_err(|e| anyhow::anyhow!("SP1 aggregation proving failed: {:?}", e))?;
    prover
        .verify(&receipt, pk.verifying_key(), None)
        .map_err(|e| anyhow::anyhow!("SP1 aggregation verification failed: {:?}", e))?;

    Ok(())
}
