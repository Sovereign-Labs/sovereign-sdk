use std::num::NonZero;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use celestia_types::nmt::Namespace;
use serde_json::{json, Value};
use sov_rollup_interface::da::{DaVerifier, RelevantBlobs};
use sov_rollup_interface::node::da::DaService;
use wiremock::matchers::{bearer_token, body_partial_json, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use crate::config::default_request_timeout_seconds;
use crate::da_service::{
    extract_relevant_blobs, get_extraction_proof, CelestiaConfig, CelestiaService,
};
use crate::test_helper::files::*;
use crate::test_helper::{raw_blob_from_data, ADDR_1, ADDR_2, ROLLUP_PARAMS_DEV};
use crate::types::{BlobWithSender, FilteredCelestiaBlock};
use crate::verifier::address::CelestiaAddress;
use crate::verifier::{CelestiaVerifier, RollupParams};

struct RpcIdEchoResponder {
    response_result: Value,
}

impl Respond for RpcIdEchoResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let request_body_json: Result<Value, _> = serde_json::from_slice(&request.body);

        let response_id = match request_body_json {
            Ok(json_value) => json_value.get("id").cloned().unwrap_or(Value::Null),
            Err(_) => Value::Null,
        };

        ResponseTemplate::new(200).set_body_json(json!({
            "id": response_id,
            "jsonrpc": "2.0",
            "result": self.response_result
        }))
    }
}

async fn setup_test_service(
    timeout_sec: Option<u64>,
    rollup_params: RollupParams,
) -> (MockServer, CelestiaConfig, CelestiaService) {
    setup_service(timeout_sec, rollup_params).await
}

// Last return value is namespace
async fn setup_service(
    timeout_sec: Option<u64>,
    params: RollupParams,
) -> (MockServer, CelestiaConfig, CelestiaService) {
    // Start a background HTTP server on a random local port
    let mock_server = MockServer::start().await;

    let address = CelestiaAddress::from_str(ADDR_1).unwrap();

    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_partial_json(json!({
            "method": "state.AccountAddress"
        })))
        .respond_with(RpcIdEchoResponder {
            response_result: json!(address.to_string()),
        })
        .mount(&mock_server)
        .await;

    let timeout_sec = timeout_sec
        .map(|t| NonZero::new(t).unwrap())
        .unwrap_or_else(default_request_timeout_seconds);
    let mut config = CelestiaConfig::dev_config(&mock_server.uri());
    config.signer_address = Some(address);
    config.request_timeout_secs = timeout_sec;

    let da_service = CelestiaService::new(config.clone(), params).await;

    (mock_server, config, da_service)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BasicJsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: serde_json::Value,
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_blob_correct() -> anyhow::Result<()> {
    let rollup_params = ROLLUP_PARAMS_DEV;
    let (mock_server, config, da_service) = setup_test_service(None, rollup_params).await;

    let blob = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

    let raw_blob = raw_blob_from_data(
        rollup_params.rollup_batch_namespace,
        blob.clone(),
        config.signer_address.as_ref().unwrap(),
    );
    let tx_config = celestia_rpc::TxConfig::default();

    let expected_tx_hash = "05D9016060072AA71B007A6CFB1B895623192D6616D513017964C3BFCD047282";
    Mock::given(method("POST"))
        .and(path("/"))
        .and(bearer_token(config.celestia_rpc_auth_token))
        .and(body_partial_json(json!({
                "method": "state.SubmitPayForBlob",
                "params": [
                    [raw_blob?],
                    tx_config
                ]
            })))
        .respond_with(RpcIdEchoResponder {
            response_result: json!({
                        "height": 30497,
                        "txhash": expected_tx_hash,
                        "codespace": "",
                        "code": 0,
                        "data": "12260A242F636F736D6F732E62616E6B2E763162657461312E4D736753656E64526573706F6E7365",
                        "raw_log": "[]",
                        "logs": [],
                        "info": "",
                        "gas_wanted": 10000000,
                        "gas_used": 69085,
                        "timestamp": "",
                        "events": [],
                    })
        })
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    let response = da_service.send_transaction(&blob).await.await??;
    assert_eq!(
        response.da_transaction_id.to_string(),
        format!("0x{expected_tx_hash}")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_blob_application_level_error() -> anyhow::Result<()> {
    let (mock_server, _config, da_service) = setup_test_service(None, ROLLUP_PARAMS_DEV).await;

    let blob: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

    // Do not check API token or expected body here.
    // Only interested in behaviour on response
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(|req: &Request| {
            let request: BasicJsonRpcRequest = serde_json::from_slice(&req.body).unwrap();
            let response_json = json!({
                "jsonrpc": "2.0",
                "id": request.id,
                "error": {
                    "code": 1,
                    "message": ": out of gas"
                }
            });
            ResponseTemplate::new(200)
                .append_header("Content-Type", "application/json")
                .set_body_json(response_json)
        })
        .up_to_n_times(4)
        .mount(&mock_server)
        .await;

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
async fn test_submit_blob_internal_server_error() -> anyhow::Result<()> {
    let (mock_server, _config, da_service) = setup_test_service(None, ROLLUP_PARAMS_DEV).await;

    let error_response = ResponseTemplate::new(500).set_body_bytes("Internal Error".as_bytes());

    let blob: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

    // Do not check API token or expected body here.
    // Only interested in behaviour on response
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(error_response)
        .up_to_n_times(4)
        .mount(&mock_server)
        .await;

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
async fn test_submit_blob_response_timeout() -> anyhow::Result<()> {
    let timeout = 1;
    let (mock_server, _config, da_service) =
        setup_test_service(Some(timeout), ROLLUP_PARAMS_DEV).await;

    let response_json = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "result": {
            "data": "122A0A282F365",
            "events": ["some event"],
            "gas_used": 70522,
            "gas_wanted": 133540,
            "height": 26,
            "logs":  [],
            "raw_log": "",
            "txhash": "C9FEFD6D35FCC73F9E7D5C74E1D33F0B7666936876F2AD75E5D0FB2944BFADF2"
        }
    });

    let error_response = ResponseTemplate::new(200)
        .append_header("Content-Type", "application/json")
        .set_delay(Duration::from_secs(timeout) + Duration::from_millis(100))
        .set_body_json(response_json);

    let blob: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

    // Do not check API token or expected body here.
    // Only interested in behaviour on response
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(error_response)
        .up_to_n_times(4)
        .mount(&mock_server)
        .await;

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

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "invalid proof self-check: InvalidRoot")]
async fn verification_fails_if_sender_changed() {
    // This is the preparation part, consider it as malicious native code:
    let mut block = with_rollup_batch_data::filtered_block();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;
    let addr_1 = CelestiaAddress::from_str(ADDR_1).unwrap();
    let addr_2 = CelestiaAddress::from_str(ADDR_2).unwrap();
    let addr_len = addr_1.as_ref().len();

    let row = block.rollup_batch_data.data.rows.get_mut(0).unwrap();
    let share = row.shares.get_mut(0).unwrap();
    let mut raw_share_1 = share.data().clone().to_vec();

    let add_pos = raw_share_1
        .windows(addr_len)
        .position(|window| window == addr_1.as_ref())
        .expect("Block should contain given address. Check source data");

    raw_share_1.splice(add_pos..add_pos + addr_len, addr_2.as_ref().iter().copied());

    let malicious_share = celestia_types::Share::from_raw(&raw_share_1).unwrap();

    row.shares[0] = malicious_share;

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
    let (_, _, da_service) = setup_test_service(None, rollup_params).await;

    let relevant_blobs = da_service.extract_relevant_blobs(&block);
    let relevant_proofs = da_service
        .get_extraction_proof(&block, &relevant_blobs)
        .await;

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
    let (_, _, da_service) = setup_test_service(None, rollup_params).await;

    let relevant_blobs = da_service.extract_relevant_blobs(&block);
    let relevant_proofs = da_service
        .get_extraction_proof(&block, &relevant_blobs)
        .await;

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
    let (_, _, da_service) = setup_test_service(None, rollup_params).await;

    let relevant_blobs = da_service.extract_relevant_blobs(&block);

    let mut relevant_proofs = da_service
        .get_extraction_proof(&block, &relevant_blobs)
        .await;
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
    let rollup_params = with_namespace_padding::ROLLUP_PARAMS;
    let (_, _, da_service) = setup_service(None, rollup_params).await;
    let relevant_blobs = da_service.extract_relevant_blobs(&block);
    assert_eq!(relevant_blobs.batch_blobs.len(), 1);
    assert_eq!(relevant_blobs.proof_blobs.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_for_padded_namespace() {
    let block: FilteredCelestiaBlock = with_namespace_padding::filtered_block();
    let rollup_params = with_namespace_padding::ROLLUP_PARAMS;
    let (_, _, da_service) = setup_service(None, rollup_params).await;

    let relevant_blobs = da_service.extract_relevant_blobs(&block);
    let relevant_proofs = da_service
        .get_extraction_proof(&block, &relevant_blobs)
        .await;

    let verifier = CelestiaVerifier::new(rollup_params);

    verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_fails_if_there_is_less_blobs_than_proofs() {
    let block = with_rollup_batch_data::filtered_block();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;
    let (_, _, da_service) = setup_test_service(None, rollup_params).await;

    let relevant_blobs = da_service.extract_relevant_blobs(&block);
    let mut relevant_proofs = da_service
        .get_extraction_proof(&block, &relevant_blobs)
        .await;

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
    let rollup_params = with_rollup_proof_data::ROLLUP_PARAMS;
    let (_, _, da_service) = setup_test_service(None, rollup_params).await;

    let relevant_blobs = da_service.extract_relevant_blobs(&block);
    let relevant_proofs = da_service
        .get_extraction_proof(&block, &relevant_blobs)
        .await;

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
async fn test_submit_proof() -> anyhow::Result<()> {
    let rollup_params = ROLLUP_PARAMS_DEV;
    let (mock_server, config, da_service) = setup_test_service(None, rollup_params).await;

    let zk_proof: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

    let raw_blob = raw_blob_from_data(
        rollup_params.rollup_proof_namespace,
        zk_proof.clone(),
        config.signer_address.as_ref().unwrap(),
    );
    let tx_config = celestia_rpc::TxConfig::default();

    Mock::given(method("POST"))
        .and(path("/"))
        .and(bearer_token(config.celestia_rpc_auth_token))
        .and(body_partial_json(json!({
                "method": "state.SubmitPayForBlob",
                "params": [
                    [raw_blob?],
                    tx_config
                ]
            })))
        .respond_with(RpcIdEchoResponder {
            response_result: json!({
                      "height": 30497,
                        "txhash": "05D9016060072AA71B007A6CFB1B895623192D6616D513017964C3BFCD047282",
                        "codespace": "",
                        "code": 0,
                        "data": "12260A242F636F736D6F732E62616E6B2E763162657461312E4D736753656E64526573706F6E7365",
                        "raw_log": "[]",
                        "logs": [],
                        "info": "",
                        "gas_wanted": 10000000,
                        "gas_used": 69085,
                        "timestamp": "",
                        "events": [],
                })
        })
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    da_service.send_proof(&zk_proof).await.await??;

    Ok(())
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

    for ((block, rollup_params, _signers), payload) in cases {
        let (_, _, da_service) = setup_test_service(None, rollup_params).await;

        let mut relevant_blobs = da_service.extract_relevant_blobs(&block);

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
// Payload was generated ages ago, so we just read it from file
#[tokio::test(flavor = "multi_thread")]
#[ignore = "should be run manually"]
async fn regenerate_test_data() -> anyhow::Result<()> {
    let client =
        jsonrpsee::http_client::HttpClientBuilder::default().build("http://127.0.0.1:26658")?;

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
    let client =
        jsonrpsee::http_client::HttpClientBuilder::default().build("http://127.0.0.1:26658")?;

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
    let client =
        jsonrpsee::http_client::HttpClientBuilder::default().build("http://127.0.0.1:26658")?;

    from_testnet_no_shares::update_test_data(&client).await;
    from_testnet_with_tail_padding::update_test_data(&client).await;
    Ok(())
}
