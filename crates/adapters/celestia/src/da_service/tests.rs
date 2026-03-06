use std::str::FromStr;
use std::time::Duration;

use crate::da_service::{extract_relevant_blobs, get_extraction_proof};
use crate::test_helper::files::*;
use crate::test_helper::{ADDR_1, ROLLUP_PARAMS_DEV};
use crate::types::{BlobWithSender, FilteredCelestiaBlock};
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
    seed[4] = (size & 0xff) as u8;
    seed[5] = ((size >> 8) & 0xff) as u8;

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

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_blob_correct() -> anyhow::Result<()> {
    let rollup_params = ROLLUP_PARAMS_DEV;
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let config = dev_node.get_config().await?;
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let da_service = CelestiaService::new(config, rollup_params, shutdown_rx).await;
    let signer = da_service
        .get_signer()
        .await
        .expect("Should be configured with signer");

    let blob = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];
    let height_before = da_service.get_head_block_header().await?.height();
    let response = da_service.send_transaction(&blob).await.await??;

    let (collected_batch_blobs, collected_proof_blobs) =
        collect_all_blobs_between(&da_service, height_before).await?;

    assert!(
        collected_proof_blobs.is_empty(),
        "Proof should not appear when sending batch blobs"
    );
    assert_single_blob(collected_batch_blobs, signer, response.blob_hash, &blob);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_proof_correct() -> anyhow::Result<()> {
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let config = dev_node.get_config().await?;
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let da_service = CelestiaService::new(config, ROLLUP_PARAMS_DEV, shutdown_rx).await;

    let zk_proof: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];
    let signer = da_service
        .get_signer()
        .await
        .expect("Should be configured with signer");

    let height_before = da_service.get_head_block_header().await?.height();
    let response = da_service.send_proof(&zk_proof).await.await??;

    let (collected_batch_blobs, collected_proof_blobs) =
        collect_all_blobs_between(&da_service, height_before).await?;

    assert!(
        collected_batch_blobs.is_empty(),
        "Batch blobs should not be sent when submitting proofs"
    );
    assert_single_blob(collected_proof_blobs, signer, response.blob_hash, &zk_proof);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multi_sender_multi_namespace_full_verification_roundtrip() -> anyhow::Result<()> {
    let _guard = sov_test_utils::initialize_logging();
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let base_config = dev_node.get_config().await?;

    let active_rollup_params = [
        RollupParams {
            rollup_batch_namespace: Namespace::const_v0(*b"n00batch00"),
            rollup_proof_namespace: Namespace::const_v0(*b"n00proof00"),
        },
        RollupParams {
            rollup_batch_namespace: Namespace::const_v0(*b"n01zzbat01"),
            rollup_proof_namespace: Namespace::const_v0(*b"n01aaprf01"),
        },
        RollupParams {
            rollup_batch_namespace: Namespace::const_v0(*b"n02batch02"),
            rollup_proof_namespace: Namespace::const_v0(*b"n02proof02"),
        },
        RollupParams {
            rollup_batch_namespace: Namespace::const_v0(*b"n03zzbat03"),
            rollup_proof_namespace: Namespace::const_v0(*b"n03aaprf03"),
        },
    ];
    let unknown_rollup_params = RollupParams {
        rollup_batch_namespace: Namespace::const_v0(*b"n99batch99"),
        rollup_proof_namespace: Namespace::const_v0(*b"n99proof99"),
    };

    let mut shutdown_senders = Vec::new();
    let mut active_services = Vec::new();
    let mut active_signers = Vec::new();

    for (sender_idx, params) in active_rollup_params.iter().enumerate() {
        let signer_private_key = dev_node.export_signer_key(sender_idx as u8).await?;
        let expected_signer = dev_node.get_signer_address(sender_idx as u8).await?;

        let mut config = base_config.clone();
        config.signer_private_key = Some(signer_private_key);

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
        shutdown_senders.push(shutdown_tx);
        let service = CelestiaService::new(config, *params, shutdown_rx).await;
        assert_eq!(
            service.get_signer().await,
            Some(expected_signer),
            "Service signer mismatch for sender index {sender_idx}"
        );
        active_services.push(service);
        active_signers.push(expected_signer);
    }

    let (unknown_shutdown_tx, unknown_shutdown_rx) = tokio::sync::watch::channel(());
    shutdown_senders.push(unknown_shutdown_tx);
    let unknown_service =
        CelestiaService::new(base_config, unknown_rollup_params, unknown_shutdown_rx).await;

    let mut verification_services = active_services.clone();
    verification_services.push(unknown_service);

    let all_rollup_params = [
        active_rollup_params[0],
        active_rollup_params[1],
        active_rollup_params[2],
        active_rollup_params[3],
        unknown_rollup_params,
    ];
    let verifiers = all_rollup_params
        .iter()
        .map(|params| CelestiaVerifier::new(*params))
        .collect::<Vec<_>>();
    let mut namespace_records = all_rollup_params
        .iter()
        .map(|_| NamespaceRecords::default())
        .collect::<Vec<_>>();

    let row_len = verification_services[0]
        .get_head_block_header()
        .await?
        .row_length();
    let batch_sizes_per_sender = (0..4)
        .map(|sender_idx| build_batch_sizes(row_len, sender_idx))
        .collect::<Vec<_>>();
    let proof_sizes_per_sender = (0..4)
        .map(|sender_idx| build_proof_sizes(row_len, sender_idx))
        .collect::<Vec<_>>();

    let mut batch_payloads = vec![Vec::new(); 4];
    let mut proof_payloads = vec![Vec::new(); 4];
    for sender_idx in 0..4 {
        batch_payloads[sender_idx] = batch_sizes_per_sender[sender_idx]
            .iter()
            .enumerate()
            .map(|(seq_idx, size)| {
                deterministic_payload(
                    *size,
                    sender_idx,
                    sender_idx,
                    SubmissionKind::Batch,
                    seq_idx,
                )
            })
            .collect();
        proof_payloads[sender_idx] = proof_sizes_per_sender[sender_idx]
            .iter()
            .enumerate()
            .map(|(seq_idx, size)| {
                deterministic_payload(
                    *size,
                    sender_idx,
                    sender_idx,
                    SubmissionKind::Proof,
                    seq_idx,
                )
            })
            .collect();
    }

    let head_before = verification_services[0]
        .get_head_block_header()
        .await?
        .height();

    let rounds = batch_payloads[0].len();
    for round_idx in 0..rounds {
        let batch_commands = (0..4)
            .map(|sender_idx| PhaseCommand {
                namespace_idx: sender_idx,
                sender: active_signers[sender_idx],
                kind: SubmissionKind::Batch,
                payload: batch_payloads[sender_idx][round_idx].clone(),
            })
            .collect::<Vec<_>>();
        execute_phase(
            &format!("round_{round_idx}_all_batch"),
            batch_commands,
            &active_services,
            &mut namespace_records,
        )
        .await?;

        if round_idx % 2 == 0 {
            let proof_commands = (0..4)
                .map(|sender_idx| PhaseCommand {
                    namespace_idx: sender_idx,
                    sender: active_signers[sender_idx],
                    kind: SubmissionKind::Proof,
                    payload: proof_payloads[sender_idx][round_idx].clone(),
                })
                .collect::<Vec<_>>();
            execute_phase(
                &format!("round_{round_idx}_all_proof"),
                proof_commands,
                &active_services,
                &mut namespace_records,
            )
            .await?;
        } else {
            let first_group = if round_idx % 4 == 1 {
                vec![0usize, 2usize]
            } else {
                vec![1usize, 3usize]
            };
            let remaining = (0..4)
                .filter(|idx| !first_group.contains(idx))
                .collect::<Vec<_>>();

            let group_commands = first_group
                .into_iter()
                .map(|sender_idx| PhaseCommand {
                    namespace_idx: sender_idx,
                    sender: active_signers[sender_idx],
                    kind: SubmissionKind::Proof,
                    payload: proof_payloads[sender_idx][round_idx].clone(),
                })
                .collect::<Vec<_>>();
            execute_phase(
                &format!("round_{round_idx}_proof_group"),
                group_commands,
                &active_services,
                &mut namespace_records,
            )
            .await?;

            for sender_idx in remaining {
                execute_phase(
                    &format!("round_{round_idx}_proof_single_sender_{sender_idx}"),
                    vec![PhaseCommand {
                        namespace_idx: sender_idx,
                        sender: active_signers[sender_idx],
                        kind: SubmissionKind::Proof,
                        payload: proof_payloads[sender_idx][round_idx].clone(),
                    }],
                    &active_services,
                    &mut namespace_records,
                )
                .await?;
            }
        }
    }

    let head_after_submit = verification_services[0]
        .get_head_block_header()
        .await?
        .height();
    let target_scan_end = head_after_submit.saturating_add(2);
    let scan_end = wait_until_head_at_least(&verification_services[0], target_scan_end).await?;
    let scan_start = head_before;

    for height in scan_start..=scan_end {
        for (namespace_idx, service) in verification_services.iter().enumerate() {
            let block = service.get_block_at(height).await?;
            let mut relevant_blobs = service.extract_relevant_blobs(&block);

            for blob in relevant_blobs.batch_blobs.iter_mut() {
                blob.advance(blob.total_len());
                namespace_records[namespace_idx]
                    .observed_batch
                    .push(BlobRecord {
                        sender: blob.sender,
                        hash: blob.hash,
                        payload: blob.verified_data().to_vec(),
                    });
            }

            for blob in relevant_blobs.proof_blobs.iter_mut() {
                blob.advance(blob.total_len());
                namespace_records[namespace_idx]
                    .observed_proof
                    .push(BlobRecord {
                        sender: blob.sender,
                        hash: blob.hash,
                        payload: blob.verified_data().to_vec(),
                    });
            }

            let relevant_proofs = service.get_extraction_proof(&block, &relevant_blobs).await;
            verifiers[namespace_idx]
                .verify_relevant_tx_list(block.header(), &relevant_blobs, relevant_proofs)
                .with_context(|| {
                    format!(
                        "Verification failed for namespace idx {namespace_idx} at height {height}",
                    )
                })?;
        }
    }

    for (namespace_idx, records) in namespace_records.iter().enumerate() {
        assert_eq!(
            multiset_counts(&records.observed_batch),
            multiset_counts(&records.expected_batch),
            "Batch mismatch for namespace idx {namespace_idx}",
        );
        assert_eq!(
            multiset_counts(&records.observed_proof),
            multiset_counts(&records.expected_proof),
            "Proof mismatch for namespace idx {namespace_idx}",
        );
    }

    for (namespace_idx, ns_record) in namespace_records.iter().enumerate().take(4) {
        assert!(
            !ns_record.observed_batch.is_empty(),
            "Expected non-empty batch observations for namespace idx {namespace_idx}"
        );
        assert!(
            !ns_record.observed_proof.is_empty(),
            "Expected non-empty proof observations for namespace idx {namespace_idx}"
        );
    }
    assert!(
        namespace_records[4].observed_batch.is_empty()
            && namespace_records[4].observed_proof.is_empty(),
        "Unknown namespace verifier should not observe blobs"
    );

    Ok(())
}

#[test]
fn bytes_for_shares_accounts_for_signer_overhead() {
    let unsigned_first = appconsts::FIRST_SPARSE_SHARE_CONTENT_SIZE;
    let signed_first = unsigned_first
        .checked_sub(appconsts::SIGNER_SIZE)
        .expect("signer size should fit into first share content size");

    assert_eq!(bytes_for_shares(1, false), unsigned_first);
    assert_eq!(bytes_for_shares(1, true), signed_first);
    assert_eq!(
        bytes_for_shares(2, true),
        signed_first + appconsts::CONTINUATION_SPARSE_SHARE_CONTENT_SIZE
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "when toxiproxy added"]
async fn test_submit_blob_application_level_error() -> anyhow::Result<()> {
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let config = dev_node.get_config().await?;
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    // TODO: disable retries
    let da_service = CelestiaService::new(config, ROLLUP_PARAMS_DEV, shutdown_rx).await;

    let blob: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

    let error = da_service
        .send_transaction(&blob)
        .await
        .await?
        .unwrap_err()
        .to_string();

    assert!(error.contains("out of gas"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "when toxiproxy added"]
async fn test_submit_blob_internal_server_error() -> anyhow::Result<()> {
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let config = dev_node.get_config().await?;
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    // TODO: disable retries
    let da_service = CelestiaService::new(config, ROLLUP_PARAMS_DEV, shutdown_rx).await;

    let blob: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

    let error = da_service
        .send_transaction(&blob)
        .await
        .await?
        .unwrap_err()
        .to_string();

    assert_eq!(
        error,
        "Celestia RPC node returned an error: Transport(Rejected { status_code: 500 })",
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "when toxiproxy added"]
async fn test_submit_blob_response_timeout() -> anyhow::Result<()> {
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let config = dev_node.get_config().await?;
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    // TODO: disable retries
    let da_service = CelestiaService::new(config, ROLLUP_PARAMS_DEV, shutdown_rx).await;

    let blob: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

    let error = da_service
        .send_transaction(&blob)
        .await
        .await?
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("RequestTimeout"),
        "Error: {error} does not contain 'Request timeout'"
    );
    Ok(())
}

async fn verification_for_correct_blocks<F1, F2>(batch_processing_fn: F1, proof_processing_fn: F2)
where
    F1: Fn(&mut BlobWithSender),
    F2: Fn(&mut BlobWithSender),
{
    let blocks = [
        with_rollup_batch_data::test_case(),
        with_rollup_proof_data::test_case(),
        without_rollup_batch_data::test_case(),
        with_several_small_rollup_batches::test_case(),
        with_several_medium_rollup_batches::test_case(),
        with_several_large_rollup_batches::test_case(),
        with_preceding_blobs_from_different_namespaces::test_case(),
        with_batch_and_proof_same_block::test_case(),
        with_namespace_padding::test_case(),
        with_parity_boundary_followed_by_namespace::test_case(),
        medium_from_devnet::test_case(),
        from_testnet::test_case(),
        from_testnet_no_shares::test_case(),
        with_mixed_v0_and_v1_blobs::test_case(),
        with_mixed_v0_and_v1_multi_v1_parity_boundary::test_case(),
        from_testnet_with_tail_padding::test_case(),
        from_mocha_shares_mismatch::test_case(),
        from_mocha_invalid_row_proof::test_case(),
        from_mocha_multi_candidate_rows_10261831::test_case(),
    ];

    for (block, rollup_params, signers) in blocks {
        let mut signers = signers.into_iter();
        let mut relevant_blobs = extract_relevant_blobs(&block);

        // Reading all blobs and proofs, so proof is built for the full data.
        {
            let blob_iters = relevant_blobs.as_iters();
            for batch in blob_iters.batch_blobs {
                let signer = signers
                    .next()
                    .expect("missing signer in test data for batch");
                assert_eq!(batch.sender, signer);
                batch_processing_fn(batch);
            }
            for proof in blob_iters.proof_blobs {
                let signer = signers
                    .next()
                    .expect("missing signer in test data for batch");
                assert_eq!(proof.sender, signer);
                proof_processing_fn(proof);
            }
        }

        let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

        let verifier = CelestiaVerifier::new(rollup_params);

        verifier
            .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_succeeds_for_correct_blocks() {
    let read_full = |blob_with_sender: &mut BlobWithSender| {
        let total_len = blob_with_sender.blob.total_len();
        blob_with_sender.blob.advance(total_len);
        let data = blob_with_sender.blob.accumulator();
        assert_eq!(data.len(), total_len);
    };
    let no_read = |blob_with_sender: &mut BlobWithSender| {
        let data = blob_with_sender.blob.accumulator();
        assert_eq!(data.len(), 0);
    };
    let single_byte = |blob_with_sender: &mut BlobWithSender| {
        let total_len = blob_with_sender.blob.total_len();
        if total_len > 0 {
            blob_with_sender.blob.advance(1);
        }
        let data = blob_with_sender.blob.accumulator();
        let expected_len = std::cmp::min(total_len, 1);
        assert_eq!(data.len(), expected_len);
    };
    let read_half = |blob_with_sender: &mut BlobWithSender| {
        let total_len = blob_with_sender.blob.total_len();
        let half_len = total_len / 2;
        blob_with_sender.blob.advance(half_len);
        let data = blob_with_sender.blob.accumulator();
        assert_eq!(data.len(), half_len);
    };

    // No read
    verification_for_correct_blocks(no_read, no_read).await;
    // Full read
    verification_for_correct_blocks(read_full, read_full).await;
    verification_for_correct_blocks(read_full, no_read).await;
    verification_for_correct_blocks(no_read, read_full).await;
    verification_for_correct_blocks(read_full, single_byte).await;
    // Single byte read
    verification_for_correct_blocks(single_byte, single_byte).await;
    // Half Read
    verification_for_correct_blocks(read_half, read_half).await;
    verification_for_correct_blocks(read_half, no_read).await;
    verification_for_correct_blocks(no_read, read_half).await;
}

#[test]
fn parity_boundary_fixture_contains_target_shape() {
    let block = with_parity_boundary_followed_by_namespace::filtered_block();
    let namespace =
        with_parity_boundary_followed_by_namespace::ROLLUP_PARAMS.rollup_batch_namespace;
    assert!(
        has_parity_boundary_followed_by_namespace(
            &block.rollup_batch_data.data,
            namespace,
            block.header.row_length(),
        ),
        "Fixture must contain a row ending at last non-parity share with parity right sibling and namespace continuation in next row",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mixed_multi_v1_parity_boundary_verification_survives_partial_reads() {
    let block = with_mixed_v0_and_v1_multi_v1_parity_boundary::filtered_block();
    let rollup_params = with_mixed_v0_and_v1_multi_v1_parity_boundary::ROLLUP_PARAMS;
    let verifier = CelestiaVerifier::new(rollup_params);
    let namespace = rollup_params.rollup_batch_namespace;
    let mut v0_starts = 0usize;
    let mut v1_starts = 0usize;
    for row in block.rollup_batch_data.data.rows() {
        for share in &row.shares {
            if share.is_parity() || crate::shares::is_tail_padding(share) {
                continue;
            }
            let Some(info_byte) = share.info_byte() else {
                continue;
            };
            if info_byte.is_sequence_start() {
                match info_byte.version() {
                    0 => v0_starts += 1,
                    1 => v1_starts += 1,
                    _ => {}
                }
            }
        }
    }

    assert!(
        has_parity_boundary_followed_by_namespace(
            &block.rollup_batch_data.data,
            namespace,
            block.header.row_length(),
        ),
        "Fixture should include parity boundary shape"
    );
    assert!(v0_starts > 0, "Fixture should include v0 blobs");
    assert!(v1_starts >= 2, "Fixture should include multiple v1 blobs");

    for read_mode in ["no_read", "single_byte", "full"] {
        let mut relevant_blobs = extract_relevant_blobs(&block);
        assert_eq!(
            relevant_blobs.proof_blobs.len(),
            0,
            "Fixture should only target batch namespace for this test"
        );
        assert!(
            relevant_blobs.batch_blobs.len() >= 2,
            "Fixture should extract multiple supported v1 blobs",
        );

        for blob in relevant_blobs.batch_blobs.iter_mut() {
            let total_len = blob.blob.total_len();
            match read_mode {
                "no_read" => {}
                "single_byte" => {
                    if total_len > 0 {
                        blob.blob.advance(1);
                    }
                }
                "full" => blob.blob.advance(total_len),
                _ => unreachable!("unexpected read mode"),
            }
        }

        let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
        assert!(
            relevant_proofs.batch.inclusion_proof.len() > relevant_blobs.batch_blobs.len(),
            "Expected skipped v0 blobs to contribute extra inclusion proofs (mode={read_mode})",
        );

        verifier
            .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
            .unwrap_or_else(|err| {
                panic!("Mixed multi-v1 verification failed in mode={read_mode}: {err}")
            });
    }
}

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "invalid proof self-check: InvalidRoot")]
async fn verification_fails_if_sender_changed() {
    // This is the preparation part, consider it as malicious native code:
    let mut block = with_rollup_batch_data::filtered_block();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;
    let addr_1 = CelestiaAddress::from_str(ADDR_1).unwrap();
    let addr_2 = CelestiaAddress::from_str(crate::test_helper::ADDR_2).unwrap();
    let addr_len = addr_1.as_ref().len();

    let serialized_ns_data = serde_json::to_string(&block.rollup_batch_data.data).unwrap();
    let row = block.rollup_batch_data.data.rows().first().unwrap();
    let share = row.shares.first().unwrap();
    // Save it to string for replacing it in JSON in the future.
    let serialized_share_before = serde_json::to_string(share).unwrap();

    let mut raw_share_1 = share.data().clone().to_vec();

    let add_pos = raw_share_1
        .windows(addr_len)
        .position(|window| window == addr_1.as_ref())
        .expect("Block should contain given address. Check source data");

    raw_share_1.splice(add_pos..add_pos + addr_len, addr_2.as_ref().iter().copied());

    let malicious_share = celestia_types::Share::from_raw(&raw_share_1).unwrap();
    let serialized_malicious_share = serde_json::to_string(&malicious_share).unwrap();

    let malicious_ns_data_json =
        serialized_ns_data.replace(&serialized_share_before, &serialized_malicious_share);
    let malicious_ns_data: NamespaceData = serde_json::from_str(&malicious_ns_data_json).unwrap();

    block.rollup_batch_data.data = malicious_ns_data;

    // This is how it is observed
    verification_error(block, "InvalidRoot", rollup_params)
        .await
        .unwrap();
}

async fn verification_error(
    block: FilteredCelestiaBlock,
    expected_err_pattern: &str,
    rollup_params: RollupParams,
) -> anyhow::Result<()> {
    let relevant_blobs = extract_relevant_blobs(&block);
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    let verifier = CelestiaVerifier::new(rollup_params);

    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();
    assert!(
        error.to_string().contains(expected_err_pattern),
        "Actual error: {error}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_fails_if_tx_missing() {
    let block = with_rollup_batch_data::filtered_block();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;

    let relevant_blobs = extract_relevant_blobs(&block);
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    let verifier = CelestiaVerifier::new(rollup_params);

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: Default::default(),
    };
    // give to verifier an empty transactions list
    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();

    assert!(
        error.to_string().contains("MoreProofsThanBlobs"),
        "Actual error: {error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "supported shares exist in namespace")]
async fn extraction_proof_fails_for_empty_blob_list_when_supported_shares_exist() {
    let block = with_mixed_v0_and_v1_blobs::filtered_block();
    let relevant_blobs = RelevantBlobs {
        batch_blobs: Default::default(),
        proof_blobs: Default::default(),
    };

    let _ = get_extraction_proof(&block, &relevant_blobs);
}

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "supported shares exist in namespace")]
async fn verification_fails_if_supported_namespace_has_empty_blob_list() {
    let block = with_rollup_batch_data::filtered_block();
    let relevant_blobs = RelevantBlobs {
        batch_blobs: Default::default(),
        proof_blobs: Default::default(),
    };

    let _ = get_extraction_proof(&block, &relevant_blobs);
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_fails_if_not_all_blobs_are_proven() {
    let block = with_rollup_batch_data::filtered_block();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;

    let relevant_blobs = extract_relevant_blobs(&block);

    let mut relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
    // drop the proof for last batch
    relevant_proofs.batch.inclusion_proof.pop();

    let verifier = CelestiaVerifier::new(rollup_params);

    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("InvalidRowProof(ProofError(Missing))"),
        "Actual error: {error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_blobs_from_padded_namespace() {
    let block: FilteredCelestiaBlock = with_namespace_padding::filtered_block();
    let relevant_blobs = extract_relevant_blobs(&block);
    assert_eq!(relevant_blobs.batch_blobs.len(), 1);
    assert_eq!(relevant_blobs.proof_blobs.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_for_padded_namespace() {
    let block: FilteredCelestiaBlock = with_namespace_padding::filtered_block();
    let rollup_params = with_namespace_padding::ROLLUP_PARAMS;

    let relevant_blobs = extract_relevant_blobs(&block);
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    let verifier = CelestiaVerifier::new(rollup_params);

    verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_fails_if_there_is_less_blobs_than_proofs() {
    let block = with_rollup_batch_data::filtered_block();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;

    let relevant_blobs = extract_relevant_blobs(&block);
    let mut relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    // push one extra blob proof
    relevant_proofs
        .batch
        .inclusion_proof
        .push(relevant_proofs.batch.inclusion_proof[0].clone());

    let verifier = CelestiaVerifier::new(rollup_params);

    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();

    assert!(
        error.to_string().contains("WrongStartShareIndex"),
        "Actual error: {error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_fails_for_incorrect_namespace() {
    let block = with_rollup_proof_data::filtered_block();

    let relevant_blobs = extract_relevant_blobs(&block);
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    // create a verifier with a different namespace than the da_service
    let verifier = CelestiaVerifier::new(RollupParams {
        rollup_proof_namespace: Namespace::new_v0(b"abc").unwrap(),
        rollup_batch_namespace: Namespace::new_v0(b"xyz").unwrap(),
    });

    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("InvalidRowProof(ProofError(Invalid(InvalidRoot)))"),
        "Actual error: {error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_payload_can_be_read_back() -> anyhow::Result<()> {
    let cases = [
        (
            with_rollup_batch_data::test_case(),
            with_rollup_batch_data::get_payload(),
        ),
        (
            with_rollup_proof_data::test_case(),
            with_rollup_proof_data::get_payload(),
        ),
        (
            with_several_small_rollup_batches::test_case(),
            with_several_small_rollup_batches::get_payload(),
        ),
        (
            with_several_medium_rollup_batches::test_case(),
            with_several_medium_rollup_batches::get_payload(),
        ),
        (
            with_several_large_rollup_batches::test_case(),
            with_several_large_rollup_batches::get_payload(),
        ),
        (
            with_namespace_padding::test_case(),
            with_namespace_padding::get_payload(),
        ),
    ];

    let assert_payload = |blobs: &mut Vec<BlobWithSender>, expected_blobs: Vec<Vec<u8>>| {
        assert_eq!(blobs.len(), expected_blobs.len());
        for (actual_batch, expected_batch) in blobs.iter_mut().zip(expected_blobs.iter()) {
            let total_len = actual_batch.blob.total_len();
            assert_eq!(total_len, expected_batch.len());
            actual_batch.blob.advance(total_len);
            let full_data = actual_batch.blob.accumulator();

            assert_eq!(full_data, expected_batch);
        }
    };

    for ((block, _rollup_params, _signers), payload) in cases {
        let mut relevant_blobs = extract_relevant_blobs(&block);

        let expected_batches = payload.batches();
        assert_payload(&mut relevant_blobs.batch_blobs, expected_batches);

        let expected_proofs = payload.proofs();
        assert_payload(&mut relevant_blobs.proof_blobs, expected_proofs);
    }

    Ok(())
}

// This test is supposed to be run manually when celestia data format is updated.
// Run celestia dev environment.
// It does not require authentication.
// The script will take payload for each test block and regenerate test data.
// The Payload was generated ages ago, so we just read it from the file
#[tokio::test(flavor = "multi_thread")]
#[ignore = "should be run manually"]
async fn regenerate_test_data() -> anyhow::Result<()> {
    let client = celestia_client::ClientBuilder::new()
        .rpc_url("http://127.0.0.1:26658")
        .grpc_url("https://127.0.0.1:9090")
        .build()
        .await?;

    let signer = CelestiaAddress::from_str(ADDR_1)?;

    let paths = [
        (with_rollup_batch_data::DATA_PATH, true),
        (without_rollup_batch_data::DATA_PATH, false),
        (with_rollup_proof_data::DATA_PATH, false),
        (with_namespace_padding::DATA_PATH, false),
    ];

    for (data_path, with_prev_header) in paths {
        let path = make_test_path(data_path);
        update_block_data(&path, &client, &signer, with_prev_header)
            .await
            .with_context(|| format!("In path {data_path}"))?;
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "should be run manually"]
async fn generate_synthetic_test_blocks() -> anyhow::Result<()> {
    let client = celestia_client::ClientBuilder::new()
        .rpc_url("http://127.0.0.1:26658")
        .grpc_url("https://127.0.0.1:9090")
        .build()
        .await?;

    let signer = CelestiaAddress::from_str(ADDR_1)?;
    with_several_small_rollup_batches::update_test_data(&client, &signer).await;
    with_several_medium_rollup_batches::update_test_data(&client, &signer).await;
    with_several_large_rollup_batches::update_test_data(&client, &signer).await;
    with_preceding_blobs_from_different_namespaces::update_test_data(&client, &signer).await?;
    with_batch_and_proof_same_block::update_test_data(&client, &signer).await;
    with_parity_boundary_followed_by_namespace::update_test_data(&client, &signer).await?;
    with_mixed_v0_and_v1_blobs::update_test_data(&client, &signer).await;
    with_mixed_v0_and_v1_multi_v1_parity_boundary::update_test_data(&client, &signer).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual fixture generation; starts dockerized celestia devnet"]
async fn generate_parity_boundary_fixture_with_docker() -> anyhow::Result<()> {
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let signer = dev_node.get_signer_address(0).await?;
    let signer_private_key = dev_node.export_signer_key(0).await?;
    let rpc_url = format!("ws://127.0.0.1:{}", dev_node.bridge_port_ipv4().await?);
    let grpc_url = format!("http://127.0.0.1:{}", dev_node.validator_port_ipv4().await?);
    let client = celestia_client::ClientBuilder::new()
        .rpc_url(&rpc_url)
        .grpc_url(&grpc_url)
        .private_key_hex(&signer_private_key)
        .build()
        .await?;

    with_parity_boundary_followed_by_namespace::update_test_data(&client, &signer).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual fixture generation; starts dockerized celestia devnet"]
async fn generate_mixed_multi_v1_parity_boundary_fixture_with_docker() -> anyhow::Result<()> {
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let signer = dev_node.get_signer_address(0).await?;
    let signer_private_key = dev_node.export_signer_key(0).await?;
    let rpc_url = format!("ws://127.0.0.1:{}", dev_node.bridge_port_ipv4().await?);
    let grpc_url = format!("http://127.0.0.1:{}", dev_node.validator_port_ipv4().await?);
    let client = celestia_client::ClientBuilder::new()
        .rpc_url(&rpc_url)
        .grpc_url(&grpc_url)
        .private_key_hex(&signer_private_key)
        .build()
        .await?;

    with_mixed_v0_and_v1_multi_v1_parity_boundary::update_test_data(&client, &signer).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "should be run manually"]
async fn generate_mocha_testnet_blocks() -> anyhow::Result<()> {
    let client = celestia_client::ClientBuilder::new()
        .rpc_url("http://127.0.0.1:26658")
        .grpc_url("https://127.0.0.1:9090")
        .build()
        .await?;

    from_testnet_no_shares::update_test_data(&client).await;
    from_testnet_with_tail_padding::update_test_data(&client).await;
    from_mocha_multi_candidate_rows_10261831::update_test_data(&client).await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual fixture refresh from Mocha RPC"]
async fn generate_mocha_multi_candidate_rows_fixture() -> anyhow::Result<()> {
    let client = celestia_client::ClientBuilder::new()
        .rpc_url("http://127.0.0.1:26658")
        .build()
        .await?;

    from_mocha_multi_candidate_rows_10261831::update_test_data(&client).await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "should be run manually if need to regenerate data"]
async fn mocha_shares_panic() -> anyhow::Result<()> {
    // Install the ring crypto provider for rustls (required for TLS connections)
    let _ = rustls::crypto::ring::default_provider().install_default();

    let client = celestia_client::ClientBuilder::new()
        .rpc_url("http://127.0.0.1:26658")
        .grpc_url("https://127.0.0.1:9090")
        .build()
        .await?;

    from_mocha_shares_mismatch::update_test_data(&client).await;

    // 10207148
    from_mocha_invalid_row_proof::update_test_data(&client).await;

    Ok(())
}
