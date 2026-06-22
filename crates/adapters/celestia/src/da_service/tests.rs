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
use sov_metrics::MonitoringConfig;
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
    let total_len = fetched_blob.total_len();
    fetched_blob.advance(total_len);
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
async fn test_submit_compressed_batch_round_trips() -> anyhow::Result<()> {
    let rollup_params = ROLLUP_PARAMS_DEV;
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let mut config = dev_node.get_config().await?;
    config.compression = crate::config::CompressOnSubmit::Lz4;
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let da_service = CelestiaService::new(config, rollup_params, shutdown_rx).await;
    let signer = da_service
        .get_signer()
        .await
        .expect("Should be configured with signer");

    // A compressible, multi-share batch (8-byte period) so the DA envelope is
    // strictly smaller than raw and spans several chunks.
    let pattern = [0xDE_u8, 0xAD, 0xBE, 0xEF, 0x12, 0x34, 0x56, 0x78];
    let blob: Vec<u8> = pattern.iter().copied().cycle().take(4000).collect();
    let height_before = da_service.get_head_block_header().await?.height();
    let response = da_service.send_transaction(&blob).await.await??;

    let (collected_batch_blobs, collected_proof_blobs) =
        collect_all_blobs_between(&da_service, height_before).await?;
    assert!(
        collected_proof_blobs.is_empty(),
        "Proof should not appear when sending batch blobs"
    );
    // The blob read back from Celestia decodes to the original rollup payload, even
    // though a smaller compressed envelope was the bytes actually posted on-DA.
    assert_single_blob(collected_batch_blobs, signer, response.blob_hash, &blob);
    Ok(())
}

/// A single, config-free reader decodes blobs from two senders that compressed with
/// *different* `compression_chunk_size` values back to the identical rollup payload.
/// This guards the failover-safety invariant: chunk size is emission-only, so a replica
/// leader configured differently from the master still produces universally-decodable
/// blobs. Each chunk carries its own framing, so decode never consults the encoder's size.
#[tokio::test(flavor = "multi_thread")]
async fn test_two_senders_different_chunk_sizes_decode_identically() -> anyhow::Result<()> {
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let base = dev_node.get_config().await?;

    // Sender A: default share-aligned chunk size.
    let mut config_a = base.clone();
    config_a.compression = crate::config::CompressOnSubmit::Lz4;
    config_a.compression_chunk_size = 482;

    // Sender B: a deliberately different chunk size (the per-chunk cap), signer key 1.
    let mut config_b = base;
    config_b.signer_private_key = Some(dev_node.export_signer_key(1).await?);
    config_b.compression = crate::config::CompressOnSubmit::Lz4;
    config_b.compression_chunk_size = 1446;

    // Compressible, multi-chunk payload at both sizes (8-byte period): ~9 chunks at 482,
    // ~3 chunks at 1446.
    let pattern = [0xDE_u8, 0xAD, 0xBE, 0xEF, 0x12, 0x34, 0x56, 0x78];
    let payload: Vec<u8> = pattern.iter().copied().cycle().take(4000).collect();

    // Guard: both configs really emit a chunked envelope (not a raw fallback), otherwise
    // the round-trip below would not exercise the chunked decode path at all.
    for cs in [482usize, 1446] {
        let da_payload = crate::envelope::encode_for_submission(&payload, true, cs);
        assert!(
            da_payload.starts_with(&crate::envelope::ENVELOPE_MAGIC)
                && da_payload.len() < payload.len(),
            "payload must compress to a chunked envelope at chunk size {cs}"
        );
    }

    let (_tx_a, rx_a) = tokio::sync::watch::channel(());
    let (_tx_b, rx_b) = tokio::sync::watch::channel(());
    let service_a = CelestiaService::new(config_a, ROLLUP_PARAMS_DEV, rx_a).await;
    let service_b = CelestiaService::new(config_b, ROLLUP_PARAMS_DEV, rx_b).await;
    let signer_a = service_a
        .get_signer()
        .await
        .expect("signer A should be configured");
    let signer_b = service_b
        .get_signer()
        .await
        .expect("signer B should be configured");

    let height_before = service_a.get_head_block_header().await?.height();
    let resp_a = service_a.send_transaction(&payload).await.await??;
    let resp_b = service_b.send_transaction(&payload).await.await??;

    // The read path is config-free, so either service reads both senders' blobs.
    let (mut batch_blobs, proof_blobs) =
        collect_all_blobs_between(&service_a, height_before).await?;
    assert!(
        proof_blobs.is_empty(),
        "no proofs expected for batch submissions"
    );
    assert_eq!(batch_blobs.len(), 2, "exactly two batch blobs expected");

    // Each sender's blob — encoded with a different chunk size — decodes to the same
    // original payload under the single, config-free reader.
    for (signer, expected_hash) in [(signer_a, resp_a.blob_hash), (signer_b, resp_b.blob_hash)] {
        let blob = batch_blobs
            .iter_mut()
            .find(|b| b.sender == signer)
            .unwrap_or_else(|| panic!("no blob found from sender {signer}"));
        assert_eq!(blob.hash, expected_hash);
        let total_len = blob.total_len();
        blob.advance(total_len);
        assert_eq!(blob.verified_data(), payload.as_slice());
    }
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
    sov_test_utils::initialize_logging();
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

#[tokio::test(flavor = "multi_thread")]
async fn test_raw_v0_and_v1_blobs_across_namespaces() -> anyhow::Result<()> {
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let base_config = dev_node.get_config().await?;

    // 5 namespaces in lexicographic order. Batch and proof are the rollup's own.
    const NS_A_PREV: Namespace = Namespace::const_v0(*b"a-prev____");
    const NS_BATCH: Namespace = Namespace::const_v0(*b"batch_____");
    const NS_MIDDLE: Namespace = Namespace::const_v0(*b"middle____");
    const NS_PROOF: Namespace = Namespace::const_v0(*b"proof_____");
    const NS_Z_LAST: Namespace = Namespace::const_v0(*b"z-last____");
    const ALL_NAMESPACES: [Namespace; 5] = [NS_A_PREV, NS_BATCH, NS_MIDDLE, NS_PROOF, NS_Z_LAST];

    let rollup_params = RollupParams {
        rollup_batch_namespace: NS_BATCH,
        rollup_proof_namespace: NS_PROOF,
    };

    // CelestiaService for reading/verifying (uses key 0).
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let _ = sov_metrics::init_metrics_tracker(&MonitoringConfig::standard(), shutdown_rx.clone());
    let da_service = CelestiaService::new(base_config.clone(), rollup_params, shutdown_rx).await;
    let verifier = CelestiaVerifier::new(rollup_params);

    // Build 6 raw clients with different signers (keys 1–6).
    let rpc_url = format!("ws://127.0.0.1:{}", dev_node.bridge_port_ipv4().await?);
    let grpc_url = format!("http://127.0.0.1:{}", dev_node.validator_port_ipv4().await?);

    let mut raw_clients = Vec::new();
    for key_idx in 1u8..=6 {
        let key_hex = dev_node.export_signer_key(key_idx).await?;
        let address = dev_node.get_signer_address(key_idx).await?;
        let client = celestia_client::ClientBuilder::new()
            .rpc_url(&rpc_url)
            .grpc_url(&grpc_url)
            .private_key_hex(&key_hex)
            .build()
            .await?;
        raw_clients.push((std::sync::Arc::new(client), address));
    }

    let head_before = da_service.get_head_block_header().await?.height();

    let mut expected_batch: Vec<BlobRecord> = Vec::new();
    let mut expected_proof: Vec<BlobRecord> = Vec::new();

    // 2 rounds: each round, each of 6 clients submits 10 blobs
    // (v0 + v1 for each of 5 namespaces).
    for round in 0..2usize {
        let mut join_set: JoinSet<anyhow::Result<()>> = JoinSet::new();

        for (client_idx, (client, signer)) in raw_clients.iter().enumerate() {
            let mut blobs_to_submit = Vec::new();
            let mut batch_records = Vec::new();
            let mut proof_records = Vec::new();

            for (ns_idx, ns) in ALL_NAMESPACES.iter().enumerate() {
                let seq_base = round * 2;
                let data_a =
                    deterministic_payload(128, ns_idx, client_idx, SubmissionKind::Batch, seq_base);
                let data_b = deterministic_payload(
                    128,
                    ns_idx,
                    client_idx,
                    SubmissionKind::Batch,
                    seq_base + 1,
                );

                // V0 blob (unsigned)
                let v0_blob =
                    celestia_types::Blob::new(*ns, data_a, None).context("v0 blob creation")?;
                blobs_to_submit.push(v0_blob);

                // V1 blob (signed)
                let v1_blob = celestia_types::Blob::new(*ns, data_b.clone(), Some(signer.0))
                    .context("v1 blob creation")?;

                // Only v1 blobs in batch/proof namespaces are expected in output.
                if *ns == NS_BATCH {
                    batch_records.push(BlobRecord {
                        sender: *signer,
                        hash: HexHash::new(*v1_blob.commitment.hash()),
                        payload: data_b,
                    });
                } else if *ns == NS_PROOF {
                    proof_records.push(BlobRecord {
                        sender: *signer,
                        hash: HexHash::new(*v1_blob.commitment.hash()),
                        payload: data_b,
                    });
                }
                blobs_to_submit.push(v1_blob);
            }

            expected_batch.extend(batch_records);
            expected_proof.extend(proof_records);

            // Submit in a spawned task so clients run in parallel within a round.
            let client_clone = client.clone();
            join_set.spawn(async move {
                let tx_config = celestia_client::tx::TxConfig::default();
                client_clone
                    .state()
                    .submit_pay_for_blob(&blobs_to_submit, tx_config)
                    .await
                    .with_context(|| {
                        format!("submit_pay_for_blob failed for client {client_idx} round {round}")
                    })?;
                Ok(())
            });
        }

        // Await all submissions in this round.
        while let Some(joined) = join_set.join_next().await {
            joined.context("join failure")??;
        }
    }

    // Scan blocks and verify.
    let target_height = da_service
        .get_head_block_header()
        .await?
        .height()
        .saturating_add(2);
    let scan_end = wait_until_head_at_least(&da_service, target_height).await?;

    let mut observed_batch: Vec<BlobRecord> = Vec::new();
    let mut observed_proof: Vec<BlobRecord> = Vec::new();

    for height in head_before..=scan_end {
        let block = da_service.get_block_at(height).await?;
        let mut relevant_blobs = da_service.extract_relevant_blobs(&block);

        for blob in relevant_blobs.batch_blobs.iter_mut() {
            blob.advance(blob.total_len());
            observed_batch.push(BlobRecord {
                sender: blob.sender,
                hash: blob.hash,
                payload: blob.verified_data().to_vec(),
            });
        }

        for blob in relevant_blobs.proof_blobs.iter_mut() {
            blob.advance(blob.total_len());
            observed_proof.push(BlobRecord {
                sender: blob.sender,
                hash: blob.hash,
                payload: blob.verified_data().to_vec(),
            });
        }

        let relevant_proofs = da_service
            .get_extraction_proof(&block, &relevant_blobs)
            .await;
        verifier
            .verify_relevant_tx_list(block.header(), &relevant_blobs, relevant_proofs)
            .with_context(|| format!("Verification failed at height {height}"))?;
    }

    assert_eq!(
        multiset_counts(&observed_batch),
        multiset_counts(&expected_batch),
        "Batch blob mismatch: v0 blobs should be excluded, all v1 batch blobs should appear"
    );
    assert_eq!(
        multiset_counts(&observed_proof),
        multiset_counts(&expected_proof),
        "Proof blob mismatch: v0 blobs should be excluded, all v1 proof blobs should appear"
    );
    assert!(
        !observed_batch.is_empty(),
        "Should have observed at least one batch blob"
    );
    assert!(
        !observed_proof.is_empty(),
        "Should have observed at least one proof blob"
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
        let total_len = blob_with_sender.total_len();
        blob_with_sender.advance(total_len);
        let data = blob_with_sender.verified_data();
        assert_eq!(data.len(), total_len);
    };
    let no_read = |blob_with_sender: &mut BlobWithSender| {
        let data = blob_with_sender.verified_data();
        assert_eq!(data.len(), 0);
    };
    let single_byte = |blob_with_sender: &mut BlobWithSender| {
        let total_len = blob_with_sender.total_len();
        if total_len > 0 {
            blob_with_sender.advance(1);
        }
        let data = blob_with_sender.verified_data();
        let expected_len = std::cmp::min(total_len, 1);
        assert_eq!(data.len(), expected_len);
    };
    let read_half = |blob_with_sender: &mut BlobWithSender| {
        let total_len = blob_with_sender.total_len();
        let half_len = total_len / 2;
        blob_with_sender.advance(half_len);
        let data = blob_with_sender.verified_data();
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
            let total_len = blob.total_len();
            match read_mode {
                "no_read" => {}
                "single_byte" => {
                    if total_len > 0 {
                        blob.advance(1);
                    }
                }
                "full" => {
                    blob.advance(total_len);
                }
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
    let block = block_with_changed_sender();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;

    // This is how it is observed
    verification_error(block, "InvalidRoot", rollup_params)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_integrity_verification_catches_proof_generation_panic() {
    let block = block_with_changed_sender();

    let error =
        super::verify_block_integrity_for_params(&block, with_rollup_batch_data::ROLLUP_PARAMS)
            .unwrap_err()
            .to_string();

    assert!(
        error.contains("Celestia block integrity verification panicked"),
        "Actual error: {error}"
    );
    assert!(
        error.contains("invalid proof self-check: InvalidRoot"),
        "Actual error: {error}"
    );
}

fn block_with_changed_sender() -> FilteredCelestiaBlock {
    // This is the preparation part, consider it as malicious native code:
    let mut block = with_rollup_batch_data::filtered_block();
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

    block
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
async fn verification_fails_if_witness_total_len_inflated() {
    verification_fails_for_forged_total_len(1_000_000).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_fails_if_witness_total_len_deflated() {
    verification_fails_for_forged_total_len(1).await;
}

/// `BlobWithSender`'s record of its total payload length backs
/// `BlobReaderTrait::total_len()`, which feeds gas accounting and size gates in the
/// STF. It is a prover-supplied witness field, so the verifier must reject any value
/// that does not match the `sequence_length` of the authenticated first share.
async fn verification_fails_for_forged_total_len(forged_sequence_len: u64) {
    let block = with_rollup_batch_data::filtered_block();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;

    let mut relevant_blobs = extract_relevant_blobs(&block);
    // Proofs are built for the honest, unread witness: one share per blob.
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    // Forge the witness through its serialized form, as a malicious prover would.
    let blob = relevant_blobs.batch_blobs.remove(0);
    let mut serialized = serde_json::to_value(&blob).unwrap();
    serialized["blob"]["inner"]["sequence_len"] = serde_json::Value::from(forged_sequence_len);
    let forged_blob: BlobWithSender = serde_json::from_value(serialized).unwrap();
    relevant_blobs.batch_blobs.insert(0, forged_blob);

    let verifier = CelestiaVerifier::new(rollup_params);
    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();

    assert!(
        error.to_string().contains("MismatchedBlobLength"),
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
async fn verification_fails_if_blob_total_len_is_forged() {
    use crate::shares::BlobIterator;
    use sov_rollup_interface::da::CountedBufReader;

    let block = with_namespace_padding::filtered_block();
    let rollup_params = with_namespace_padding::ROLLUP_PARAMS;

    let mut relevant_blobs = extract_relevant_blobs(&block);
    // The length that the DA shares actually prove for this blob.
    let proven_len = relevant_blobs.batch_blobs[0].total_len();

    // Build the inclusion proof from the honest blob first, then swap in a blob whose
    // witness-derived `total_len()` disagrees with the proven sequence length. The verifier
    // only authenticates the accumulator *content*, so without the cross-check a malicious
    // prover could smuggle in an arbitrary `total_len()` (used downstream by sov-blob-storage
    // for the malformed-blob slash decision, size limits, and deserialization gas charging).
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
    let forged_len = proven_len + 7;
    relevant_blobs.batch_blobs[0].blob =
        CountedBufReader::new(BlobIterator::with_forged_len(forged_len));

    let verifier = CelestiaVerifier::new(rollup_params);
    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();

    assert!(
        error.to_string().contains("MismatchedBlobLength"),
        "Expected the verifier to reject a forged total_len, got: {error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_rejects_compressed_envelope_witness_without_header() -> anyhow::Result<()> {
    use crate::shares::BlobIterator;
    use sov_rollup_interface::da::CountedBufReader;

    let rollup_params = ROLLUP_PARAMS_DEV;
    let dev_node = crate::test_helper::docker::CelestiaDevNode::start().await?;
    let mut config = dev_node.get_config().await?;
    config.compression = crate::config::CompressOnSubmit::Lz4;
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let da_service = CelestiaService::new(config, rollup_params, shutdown_rx).await;

    // A compressible batch whose DA envelope is shorter than the rollup payload.
    let pattern = [0xDE_u8, 0xAD, 0xBE, 0xEF, 0x12, 0x34, 0x56, 0x78];
    let rollup_blob: Vec<u8> = pattern.iter().copied().cycle().take(4000).collect();
    let height_before = da_service.get_head_block_header().await?.height();
    let response = da_service.send_transaction(&rollup_blob).await.await??;
    let height_after = da_service
        .get_head_block_header()
        .await?
        .height()
        .saturating_add(1);

    let mut matching_block = None;
    for height in height_before..=height_after {
        let block = da_service.get_block_at(height).await?;
        let relevant_blobs = extract_relevant_blobs(&block);
        if relevant_blobs
            .batch_blobs
            .iter()
            .any(|blob| blob.hash == response.blob_hash)
        {
            matching_block = Some(block);
            break;
        }
    }
    let block = matching_block.context("submitted compressed blob was not found")?;

    let mut relevant_blobs = extract_relevant_blobs(&block);
    let blob_idx = relevant_blobs
        .batch_blobs
        .iter()
        .position(|blob| blob.hash == response.blob_hash)
        .context("submitted compressed blob was not extracted")?;

    let da_len = relevant_blobs.batch_blobs[blob_idx].compressed_total_len();
    assert!(
        relevant_blobs.batch_blobs[blob_idx]
            .compressed_verified_data()
            .starts_with(&crate::envelope::ENVELOPE_MAGIC),
        "native extraction should eagerly authenticate the envelope header",
    );
    assert_ne!(
        da_len,
        rollup_blob.len(),
        "fixture must distinguish DA and rollup lengths",
    );
    assert_eq!(
        relevant_blobs.batch_blobs[blob_idx].clone().total_len(),
        rollup_blob.len(),
        "honest native witness should expose the envelope rollup length",
    );

    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
    relevant_blobs.batch_blobs[blob_idx].blob =
        CountedBufReader::new(BlobIterator::with_forged_len(da_len));
    assert_eq!(
        relevant_blobs.batch_blobs[blob_idx]
            .compressed_verified_data()
            .len(),
        0,
        "forged witness omits the authenticated envelope header",
    );
    assert_eq!(
        relevant_blobs.batch_blobs[blob_idx].total_len(),
        da_len,
        "without the header, the witness is classified as legacy and exposes DA length",
    );

    let verifier = CelestiaVerifier::new(rollup_params);
    verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .expect_err("verifier accepted an envelope witness that omitted the header");

    Ok(())
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
            let total_len = actual_batch.total_len();
            assert_eq!(total_len, expected_batch.len());
            actual_batch.advance(total_len);
            let full_data = actual_batch.verified_data();

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
async fn generate_mainnet_real_rollup_data() -> anyhow::Result<()> {
    let client = celestia_client::ClientBuilder::new()
        .rpc_url("http://celestia-mainnet-da.itrocket.net:26658")
        .build()
        .await?;

    from_mainnet_real_rollup_average::update_test_data(&client).await;
    from_mainnet_real_rollup_p99::update_test_data(&client).await;
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
