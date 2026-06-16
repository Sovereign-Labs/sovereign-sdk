use std::fmt::Write as _;
use std::path::Path;

use anyhow::{ensure, Context};
use clap::Args;
use sov_celestia_adapter::types::{BlobWithSender, Namespace};
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::{
    extract_relevant_blobs, filtered_block_from_json_path, get_extraction_proof,
};
use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait, RelevantBlobs};
use sp1_sdk::blocking::{Prover, ProverClient, SP1Stdin};

use crate::load_guest_elf;

const GUEST_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/guest-celestia/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-microbench-guest-celestia",
);

const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../adapters/celestia/test_data/block_mocha_multi_candidate_rows_10261831",
);

const FULL_BLOCK_REGION: &str = "celestia_full_block";
const VERIFY_REGION: &str = "celestia_verify";

const ROLLUP_PARAMS: RollupParams = RollupParams {
    rollup_batch_namespace: Namespace::const_v0([0, 0, 10, 117, 61, 127, 167, 56, 47, 69]),
    rollup_proof_namespace: Namespace::const_v0([115, 111, 118, 45, 116, 101, 115, 116, 45, 112]),
};

type CelestiaGuestOutput = ([u8; 32], u64, u64, u64, u64, u64);

#[derive(Args, Debug)]
pub struct CelestiaArgs {}

pub fn run(_args: CelestiaArgs) -> anyhow::Result<()> {
    let elf = load_guest_elf(GUEST_ELF_PATH)?;
    let client = ProverClient::from_env();

    let block = filtered_block_from_json_path(
        ROLLUP_PARAMS.rollup_batch_namespace,
        ROLLUP_PARAMS.rollup_proof_namespace,
        Path::new(FIXTURE_PATH),
    )
    .context("failed to load Celestia fixture")?;
    let block_bytes = bincode::serialize(&block).context("failed to serialize fixture block")?;
    let relevant_blobs = extract_relevant_blobs(&block);
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
    let expected_output = guest_output(&block, block_bytes.len() as u64, &relevant_blobs);

    println!(
        "[run] celestia fixture={} serialized_bytes={}",
        FIXTURE_PATH,
        block_bytes.len()
    );

    let mut stdin = SP1Stdin::new();
    stdin.write_vec(block_bytes);
    stdin.write(&relevant_blobs);
    stdin.write(&relevant_proofs);

    let (mut public_values, report) = client
        .execute(elf, stdin)
        .run()
        .context("sp1 execute failed for Celestia verifier bench")?;
    let output: CelestiaGuestOutput = public_values.read();
    ensure!(
        output == expected_output,
        "guest output mismatch: expected {:?}, got {:?}",
        expected_output,
        output
    );

    let prover_gas = report
        .gas()
        .context("prover gas not available; ProverClient may have disabled gas calculation")?;
    let total_cycles = report.total_instruction_count();
    let full_block_cycles = report
        .cycle_tracker
        .get(FULL_BLOCK_REGION)
        .copied()
        .unwrap_or(0);
    let verify_cycles = report
        .cycle_tracker
        .get(VERIFY_REGION)
        .copied()
        .unwrap_or(0);
    let full_block_invocations = report
        .invocation_tracker
        .get(FULL_BLOCK_REGION)
        .copied()
        .unwrap_or(0);
    let verify_invocations = report
        .invocation_tracker
        .get(VERIFY_REGION)
        .copied()
        .unwrap_or(0);

    println!("\n=== celestia fixture ===");
    println!("path                   = {FIXTURE_PATH}");
    println!(
        "batch namespace        = {}",
        namespace_label(&ROLLUP_PARAMS.rollup_batch_namespace)
    );
    println!(
        "proof namespace        = {}",
        namespace_label(&ROLLUP_PARAMS.rollup_proof_namespace)
    );
    println!("block hash             = 0x{}", hex_bytes(&output.0));
    println!("serialized block bytes = {}", output.1);
    println!("batch blobs            = {}", output.2);
    println!("proof blobs            = {}", output.3);
    println!("batch payload bytes    = {}", output.4);
    println!("proof payload bytes    = {}", output.5);

    println!("\n=== raw measurements ===");
    println!("prover gas             = {prover_gas}");
    println!("total cycles           = {total_cycles}");
    println!("full-block cycles      = {full_block_cycles}");
    println!("verify cycles          = {verify_cycles}");
    println!("full-block invocations = {full_block_invocations}");
    println!("verify invocations     = {verify_invocations}");

    Ok(())
}

fn guest_output(
    block: &sov_celestia_adapter::types::FilteredCelestiaBlock,
    block_bytes: u64,
    relevant_blobs: &RelevantBlobs<BlobWithSender>,
) -> CelestiaGuestOutput {
    let block_hash = *block.header().hash().inner();
    let batch_blob_count = relevant_blobs.batch_blobs.len() as u64;
    let proof_blob_count = relevant_blobs.proof_blobs.len() as u64;
    let batch_bytes = relevant_blobs
        .batch_blobs
        .iter()
        .map(BlobReaderTrait::total_len)
        .sum::<usize>() as u64;
    let proof_bytes = relevant_blobs
        .proof_blobs
        .iter()
        .map(BlobReaderTrait::total_len)
        .sum::<usize>() as u64;

    (
        block_hash,
        block_bytes,
        batch_blob_count,
        proof_blob_count,
        batch_bytes,
        proof_bytes,
    )
}

fn namespace_label(namespace: &Namespace) -> String {
    let bytes = namespace.as_bytes();
    let hex = hex_bytes(bytes);
    if bytes.iter().all(u8::is_ascii_graphic) {
        format!("0x{hex} ({})", String::from_utf8_lossy(bytes))
    } else {
        format!("0x{hex}")
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to string cannot fail");
    }
    output
}
