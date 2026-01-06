use std::str::FromStr;

use crate::da_service::{extract_relevant_blobs, get_extraction_proof};
use crate::test_helper::files::*;
use crate::test_helper::{ADDR_1, ROLLUP_PARAMS_DEV};
use crate::types::{BlobWithSender, FilteredCelestiaBlock};
use crate::verifier::address::CelestiaAddress;
use crate::verifier::{CelestiaVerifier, RollupParams};
use crate::CelestiaService;
use anyhow::Context;
use celestia_types::nmt::Namespace;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait, DaVerifier, RelevantBlobs};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::da::SlotData;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BasicJsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: serde_json::Value,
}

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
    assert_eq!(1, blobs.len());
    let mut fetched_blob = blobs.pop().unwrap();
    assert_eq!(fetched_blob.sender, expected_signer);
    assert_eq!(fetched_blob.hash, expected_hash);
    fetched_blob.blob.advance(fetched_blob.total_len());
    assert_eq!(fetched_blob.verified_data(), expected_data);
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
        "Celestia RPC node returned an error: Transport(Rejected { status_code: 500 })",
        error
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
        medium_from_devnet::test_case(),
        from_testnet::test_case(),
        from_testnet_no_shares::test_case(),
        with_mixed_v0_and_v1_blobs::test_case(),
        from_testnet_with_tail_padding::test_case(),
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
                assert_eq!(signer, batch.sender);
                batch_processing_fn(batch);
            }
            for proof in blob_iters.proof_blobs {
                let signer = signers
                    .next()
                    .expect("missing signer in test data for batch");
                assert_eq!(signer, proof.sender);
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

// #[tokio::test(flavor = "multi_thread")]
// #[should_panic(expected = "invalid proof self-check: InvalidRoot")]
// async fn verification_fails_if_sender_changed() {
//     // This is the preparation part, consider it as malicious native code:
//     // TODO: Find a way to test it differently
//     // let mut block = with_rollup_batch_data::filtered_block();
//     // let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;
//     // let addr_1 = CelestiaAddress::from_str(ADDR_1).unwrap();
//     // let addr_2 = CelestiaAddress::from_str(crate::test_helper::ADDR_2).unwrap();
//     // let addr_len = addr_1.as_ref().len();
//     //
//     // let row = block.rollup_batch_data.data.rows().get(0).unwrap();
//     // let share = row.shares.get(0).unwrap();
//     // let mut raw_share_1 = share.data().clone().to_vec();
//     //
//     // let add_pos = raw_share_1
//     //     .windows(addr_len)
//     //     .position(|window| window == addr_1.as_ref())
//     //     .expect("Block should contain given address. Check source data");
//     //
//     // raw_share_1.splice(add_pos..add_pos + addr_len, addr_2.as_ref().iter().copied());
//     //
//     // let malicious_share = celestia_types::Share::from_raw(&raw_share_1).unwrap();
//     //
//     // row.shares[0] = malicious_share;
//     //
//     // // This is how it is observed
//     // verification_error(block, "InvalidRoot", rollup_params)
//     //     .await
//     //     .unwrap();
// }
//
// async fn verification_error(
//     block: FilteredCelestiaBlock,
//     expected_err_pattern: &str,
//     rollup_params: RollupParams,
// ) -> anyhow::Result<()> {
//     let relevant_blobs = extract_relevant_blobs(&block);
//     let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
//
//     let verifier = CelestiaVerifier::new(rollup_params);
//
//     let error = verifier
//         .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
//         .unwrap_err();
//     assert!(
//         error.to_string().contains(expected_err_pattern),
//         "Actual error: {error}"
//     );
//     Ok(())
// }

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
        error
            .to_string()
            .contains("IncompleteNamespace(ProofError(Invalid(WrongAmountOfLeavesProvided)))"),
        "Actual error: {error}"
    );
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
    with_mixed_v0_and_v1_blobs::update_test_data(&client, &signer).await;
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
    Ok(())
}
