use std::path::Path;

use anyhow::{ensure, Context};
use clap::Args;
use sov_celestia_adapter::types::{BlobWithSender, Namespace};
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

const TEST_DATA_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../adapters/celestia/test_data",
);

const FULL_BLOCK_REGION: &str = "celestia_full_block";
const VERIFY_REGION: &str = "celestia_verify";

// v0 namespace ids (the 10-byte suffix passed to `Namespace::const_v0`). All cases share the
// `sov-test-p` proof namespace; only the batch namespace differs between the mocha fixture and
// the devnet-generated mainnet fixtures.
const MOCHA_BATCH_NS: [u8; 10] = [0, 0, 10, 117, 61, 127, 167, 56, 47, 69];
const DEV_BATCH_NS: [u8; 10] = *b"\0\0sov-test";
const PROOF_NS: [u8; 10] = *b"sov-test-p";

type CelestiaGuestOutput = ([u8; 32], u64, u64, u64, u64, u64);

/// A single benchmark case. "Compression off vs LZ4" is encoded in which fixture directory is
/// loaded — compression is baked into the posted Celestia shares, not a runtime flag — so the
/// off/LZ4 variants are distinct fixtures sharing the same namespace.
struct Case {
    label: &'static str,
    fixture_dir: &'static str,
    batch_ns_id: [u8; 10],
    proof_ns_id: [u8; 10],
}

const CASES: &[Case] = &[
    Case {
        label: "mocha",
        fixture_dir: "block_mocha_multi_candidate_rows_10261831",
        batch_ns_id: MOCHA_BATCH_NS,
        proof_ns_id: PROOF_NS,
    },
    Case {
        label: "mainnet_avg_off",
        fixture_dir: "block_mainnet_real_rollup_average_off",
        batch_ns_id: DEV_BATCH_NS,
        proof_ns_id: PROOF_NS,
    },
    Case {
        label: "mainnet_avg_lz4",
        fixture_dir: "block_mainnet_real_rollup_average_lz4",
        batch_ns_id: DEV_BATCH_NS,
        proof_ns_id: PROOF_NS,
    },
    Case {
        label: "mainnet_p99_off",
        fixture_dir: "block_mainnet_real_rollup_p99_off",
        batch_ns_id: DEV_BATCH_NS,
        proof_ns_id: PROOF_NS,
    },
    Case {
        label: "mainnet_p99_lz4",
        fixture_dir: "block_mainnet_real_rollup_p99_lz4",
        batch_ns_id: DEV_BATCH_NS,
        proof_ns_id: PROOF_NS,
    },
];

#[derive(Args, Debug)]
pub struct CelestiaArgs {}

struct Row {
    label: &'static str,
    serialized_bytes: u64,
    batch_blobs: u64,
    batch_payload_bytes: u64,
    total_cycles: u64,
    full_block_cycles: u64,
    verify_cycles: u64,
    prover_gas: u64,
}

pub fn run(_args: CelestiaArgs) -> anyhow::Result<()> {
    let elf = load_guest_elf(GUEST_ELF_PATH)?;
    let client = ProverClient::from_env();

    let mut rows = Vec::with_capacity(CASES.len());
    for case in CASES {
        println!(
            "[run] celestia case={} fixture={}",
            case.label, case.fixture_dir
        );

        let fixture_path = Path::new(TEST_DATA_DIR).join(case.fixture_dir);
        let batch_ns = Namespace::const_v0(case.batch_ns_id);
        let proof_ns = Namespace::const_v0(case.proof_ns_id);

        let block = filtered_block_from_json_path(batch_ns, proof_ns, &fixture_path)
            .with_context(|| format!("failed to load Celestia fixture for case {}", case.label))?;
        let block_bytes =
            bincode::serialize(&block).context("failed to serialize fixture block")?;
        let relevant_blobs = extract_relevant_blobs(&block);
        let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
        let expected_output = guest_output(&block, block_bytes.len() as u64, &relevant_blobs);

        // The namespace ids are written first; the guest reads them before the block bytes.
        let mut stdin = SP1Stdin::new();
        stdin.write(&case.batch_ns_id);
        stdin.write(&case.proof_ns_id);
        stdin.write_vec(block_bytes);
        stdin.write(&relevant_blobs);
        stdin.write(&relevant_proofs);

        let (mut public_values, report) = client
            .execute(elf.clone(), stdin)
            .run()
            .with_context(|| format!("sp1 execute failed for case {}", case.label))?;
        let output: CelestiaGuestOutput = public_values.read();
        ensure!(
            output == expected_output,
            "guest output mismatch for case {}: expected {:?}, got {:?}",
            case.label,
            expected_output,
            output
        );

        let prover_gas = report
            .gas()
            .context("prover gas not available; ProverClient may have disabled gas calculation")?;
        rows.push(Row {
            label: case.label,
            serialized_bytes: output.1,
            batch_blobs: output.2,
            batch_payload_bytes: output.4,
            total_cycles: report.total_instruction_count(),
            full_block_cycles: report
                .cycle_tracker
                .get(FULL_BLOCK_REGION)
                .copied()
                .unwrap_or(0),
            verify_cycles: report
                .cycle_tracker
                .get(VERIFY_REGION)
                .copied()
                .unwrap_or(0),
            prover_gas,
        });
    }

    print_table(&rows);
    Ok(())
}

fn print_table(rows: &[Row]) {
    println!("\n=== celestia verifier bench ===");
    let header = format!(
        "{:<16}  {:>12}  {:>11}  {:>13}  {:>14}  {:>15}  {:>13}  {:>14}",
        "case",
        "ser bytes",
        "batch blobs",
        "batch payload",
        "total cycles",
        "full-blk cycles",
        "verify cycles",
        "prover gas",
    );
    println!("{header}");
    for r in rows {
        println!(
            "{:<16}  {:>12}  {:>11}  {:>13}  {:>14}  {:>15}  {:>13}  {:>14}",
            r.label,
            r.serialized_bytes,
            r.batch_blobs,
            r.batch_payload_bytes,
            r.total_cycles,
            r.full_block_cycles,
            r.verify_cycles,
            r.prover_gas,
        );
    }
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
