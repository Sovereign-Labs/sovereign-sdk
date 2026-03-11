use std::str::FromStr;
use std::time::Duration;

use crate::da_service::{build_extraction_proof, extract_relevant_blobs, get_extraction_proof};
use crate::test_helper::files::*;
use crate::test_helper::{ADDR_1, ROLLUP_PARAMS_DEV};
use crate::test_support::assert_subproof_start_indices_align;
use crate::types::{BlobWithSender, FilteredCelestiaBlock, NamespaceBoundaryProof};
use crate::verifier::address::CelestiaAddress;
use crate::verifier::{CelestiaVerifier, RollupParams};
use crate::CelestiaService;
use anyhow::Context;
use celestia_types::consts::appconsts;
use celestia_types::namespace_data::NamespaceData;
use celestia_types::nmt::Namespace;
use rand::{RngCore, SeedableRng};
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait, DaVerifier, RelevantBlobs};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::da::SlotData;
use tokio::task::JoinSet;

mod adversarial_tampering;
mod fixture_generation;
mod multirow_absence_fixtures;
mod multirow_absence_spec;
mod positive_verification;
mod rejection;
mod submission;

type BlobReader = fn(&mut BlobWithSender);

async fn collect_all_blobs_between(
    da_service: &CelestiaService,
    height_before: u64,
) -> anyhow::Result<(Vec<BlobWithSender>, Vec<BlobWithSender>)> {
    // Adding one more height to the current head to accommodate for blob inclusion.
    // Even though by the time submitPayForBlob has returned it should be included, we observed flakiness.
    let height_after = da_service
        .get_head_block_header()
        .await?
        .height()
        .saturating_add(1);
    let mut collected_batch_blobs = Vec::new();
    let mut collected_proof_blobs = Vec::new();

    for height in height_before..=height_after {
        let block = da_service.get_block_at(height).await?;
        // Tiny self check.
        assert_eq!(block.header().height(), height);
        let relevant_blobs = da_service.extract_relevant_blobs(&block);
        let RelevantBlobs {
            batch_blobs,
            proof_blobs,
        } = relevant_blobs;
        collected_batch_blobs.extend(batch_blobs);
        collected_proof_blobs.extend(proof_blobs);
    }

    Ok((collected_batch_blobs, collected_proof_blobs))
}

fn assert_single_blob(
    mut blobs: Vec<BlobWithSender>,
    expected_signer: CelestiaAddress,
    expected_hash: HexHash,
    expected_data: &[u8],
) {
    assert_eq!(blobs.len(), 1);
    let mut fetched_blob = blobs.pop().unwrap();
    assert_eq!(fetched_blob.sender, expected_signer);
    assert_eq!(fetched_blob.hash, expected_hash);
    fetched_blob.blob.advance(fetched_blob.total_len());
    assert_eq!(fetched_blob.verified_data(), expected_data);
}

#[derive(Debug, Clone, Copy)]
enum SubmissionKind {
    Batch,
    Proof,
}

impl SubmissionKind {
    fn as_byte(self) -> u8 {
        match self {
            Self::Batch => 0,
            Self::Proof => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BlobRecord {
    sender: CelestiaAddress,
    hash: HexHash,
    payload: Vec<u8>,
}

#[derive(Debug, Clone)]
struct PhaseCommand {
    namespace_idx: usize,
    sender: CelestiaAddress,
    kind: SubmissionKind,
    payload: Vec<u8>,
}

#[derive(Debug, Default)]
struct NamespaceRecords {
    expected_batch: Vec<BlobRecord>,
    expected_proof: Vec<BlobRecord>,
    observed_batch: Vec<BlobRecord>,
    observed_proof: Vec<BlobRecord>,
}

fn bytes_for_shares(share_count: usize, has_signer: bool) -> usize {
    crate::shares::payload_bytes_for_shares_with_signer(share_count, has_signer)
}

fn deterministic_payload(
    size: usize,
    namespace_idx: usize,
    sender_idx: usize,
    kind: SubmissionKind,
    seq_idx: usize,
) -> Vec<u8> {
    let mut seed = [0u8; 32];
    seed[0] = namespace_idx as u8;
    seed[1] = sender_idx as u8;
    seed[2] = kind.as_byte();
    seed[3] = seq_idx as u8;
    seed[4..6].copy_from_slice(&(size as u16).to_le_bytes());

    let mut payload = vec![0u8; size];
    let mut rng = rand::rngs::SmallRng::from_seed(seed);
    rng.fill_bytes(&mut payload);

    if !payload.is_empty() {
        payload[0] = namespace_idx as u8;
    }
    if payload.len() > 1 {
        payload[1] = sender_idx as u8;
    }
    if payload.len() > 2 {
        payload[2] = kind.as_byte();
    }
    if payload.len() > 3 {
        payload[3] = seq_idx as u8;
    }

    payload
}

fn build_batch_sizes(row_len: usize, sender_idx: usize) -> [usize; 4] {
    // This test configures all active senders with a private key, so blobs are v1 signed.
    let has_signer = true;
    let small_exact = bytes_for_shares(1, has_signer);
    let small_overflow = small_exact.saturating_add(1);
    let power_of_two = if sender_idx.is_multiple_of(2) {
        bytes_for_shares(4, has_signer)
    } else {
        bytes_for_shares(8, has_signer).saturating_add(1)
    };
    let row_case = match sender_idx {
        0 => bytes_for_shares(row_len.saturating_sub(1).max(1), has_signer),
        1 => bytes_for_shares(row_len, has_signer),
        2 => bytes_for_shares(row_len, has_signer).saturating_add(1),
        3 => bytes_for_shares(row_len.saturating_add(1), has_signer),
        _ => unreachable!("sender_idx should be in range [0..4)"),
    };

    [small_exact, small_overflow, power_of_two, row_case]
}

fn build_proof_sizes(row_len: usize, sender_idx: usize) -> [usize; 4] {
    // This test configures all active senders with a private key, so blobs are v1 signed.
    let has_signer = true;
    let small_exact = bytes_for_shares(1, has_signer);
    let small_overflow = small_exact.saturating_add(1);
    let power_of_two = if sender_idx.is_multiple_of(2) {
        bytes_for_shares(8, has_signer)
    } else {
        bytes_for_shares(4, has_signer).saturating_add(1)
    };
    let row_case = match sender_idx {
        0 => bytes_for_shares(row_len, has_signer),
        1 => bytes_for_shares(row_len.saturating_add(1), has_signer),
        2 => bytes_for_shares(row_len.saturating_sub(1).max(1), has_signer),
        3 => bytes_for_shares(row_len, has_signer).saturating_add(1),
        _ => unreachable!("sender_idx should be in range [0..4)"),
    };

    [small_exact, small_overflow, power_of_two, row_case]
}

fn multiset_counts(records: &[BlobRecord]) -> std::collections::HashMap<BlobRecord, usize> {
    let mut output = std::collections::HashMap::new();
    for record in records.iter().cloned() {
        *output.entry(record).or_insert(0) += 1;
    }
    output
}

async fn execute_phase(
    phase_name: &str,
    commands: Vec<PhaseCommand>,
    services: &[CelestiaService],
    namespace_records: &mut [NamespaceRecords],
) -> anyhow::Result<()> {
    let mut join_set: JoinSet<anyhow::Result<(PhaseCommand, HexHash)>> = JoinSet::new();
    for command in commands {
        let service = services[command.namespace_idx].clone();
        join_set.spawn(async move {
            let submit_result = match command.kind {
                SubmissionKind::Batch => service.send_transaction(&command.payload).await,
                SubmissionKind::Proof => service.send_proof(&command.payload).await,
            };
            let receipt = submit_result
                .await
                .context("Submission receiver has been dropped")??;
            Ok((command, receipt.blob_hash))
        });
    }

    while let Some(joined) = join_set.join_next().await {
        let (command, blob_hash) =
            joined.with_context(|| format!("Submission task join failure for {phase_name}"))??;
        let PhaseCommand {
            namespace_idx,
            sender,
            kind,
            payload,
        } = command;
        let record = BlobRecord {
            sender,
            hash: blob_hash,
            payload,
        };

        match kind {
            SubmissionKind::Batch => namespace_records[namespace_idx].expected_batch.push(record),
            SubmissionKind::Proof => namespace_records[namespace_idx].expected_proof.push(record),
        }
    }

    Ok(())
}

async fn wait_until_head_at_least(
    service: &CelestiaService,
    target_height: u64,
) -> anyhow::Result<u64> {
    const MAX_POLLS: usize = 120;
    const POLL_INTERVAL: Duration = Duration::from_millis(250);

    let mut head = service.get_head_block_header().await?.height();
    for _ in 0..MAX_POLLS {
        if head >= target_height {
            return Ok(head);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        head = service.get_head_block_header().await?.height();
    }

    Err(anyhow::anyhow!(
        "Timed out waiting for head >= target height: target={target_height}, head={head}, polls={MAX_POLLS}, interval_ms={}",
        POLL_INTERVAL.as_millis()
    ))
}

fn read_full_blob(blob_with_sender: &mut BlobWithSender) {
    let total_len = blob_with_sender.blob.total_len();
    blob_with_sender.blob.advance(total_len);
    let data = blob_with_sender.blob.accumulator();
    assert_eq!(data.len(), total_len);
}

fn read_no_blob(blob_with_sender: &mut BlobWithSender) {
    let data = blob_with_sender.blob.accumulator();
    assert_eq!(data.len(), 0);
}

fn read_single_byte_blob(blob_with_sender: &mut BlobWithSender) {
    let total_len = blob_with_sender.blob.total_len();
    if total_len > 0 {
        blob_with_sender.blob.advance(1);
    }
    let data = blob_with_sender.blob.accumulator();
    let expected_len = std::cmp::min(total_len, 1);
    assert_eq!(data.len(), expected_len);
}

fn read_half_blob(blob_with_sender: &mut BlobWithSender) {
    let total_len = blob_with_sender.blob.total_len();
    let half_len = total_len / 2;
    blob_with_sender.blob.advance(half_len);
    let data = blob_with_sender.blob.accumulator();
    assert_eq!(data.len(), half_len);
}

fn apply_blob_readers(
    relevant_blobs: &mut RelevantBlobs<BlobWithSender>,
    batch_reader: BlobReader,
    proof_reader: BlobReader,
) {
    for blob in &mut relevant_blobs.batch_blobs {
        batch_reader(blob);
    }
    for blob in &mut relevant_blobs.proof_blobs {
        proof_reader(blob);
    }
}

fn read_fixture_blobs(
    relevant_blobs: &mut RelevantBlobs<BlobWithSender>,
    signers: impl IntoIterator<Item = CelestiaAddress>,
    batch_reader: BlobReader,
    proof_reader: BlobReader,
) {
    let mut signers = signers.into_iter();
    let blob_iters = relevant_blobs.as_iters();

    for batch in blob_iters.batch_blobs {
        let signer = signers
            .next()
            .expect("missing signer in test data for batch");
        assert_eq!(signer, batch.sender);
        batch_reader(batch);
    }
    for proof in blob_iters.proof_blobs {
        let signer = signers
            .next()
            .expect("missing signer in test data for proof");
        assert_eq!(signer, proof.sender);
        proof_reader(proof);
    }
}

fn assert_missing_supported_namespace_error(
    block: &FilteredCelestiaBlock,
    relevant_blobs: RelevantBlobs<BlobWithSender>,
) {
    let err = build_extraction_proof(block, &relevant_blobs).unwrap_err();
    assert!(
        matches!(
            err,
            crate::types::ExtractionProofError::MissingBlobsForSupportedNamespace { .. }
        ),
        "Actual error: {err}"
    );
}

fn verify_fixture_with_readers(
    fixture: (FilteredCelestiaBlock, RollupParams, Vec<CelestiaAddress>),
    batch_processing_fn: BlobReader,
    proof_processing_fn: BlobReader,
) {
    let (block, rollup_params, signers) = fixture;
    let mut relevant_blobs = extract_relevant_blobs(&block);
    read_fixture_blobs(
        &mut relevant_blobs,
        signers,
        batch_processing_fn,
        proof_processing_fn,
    );

    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
    let verifier = CelestiaVerifier::new(rollup_params);
    verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap();
}
