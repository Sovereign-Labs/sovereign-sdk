use super::*;

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

    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let _ = sov_metrics::init_metrics_tracker(
        &sov_metrics::MonitoringConfig::standard(),
        shutdown_rx.clone(),
    );
    let da_service = CelestiaService::new(base_config.clone(), rollup_params, shutdown_rx).await;
    let verifier = CelestiaVerifier::new(rollup_params);

    // Build 6 raw clients with different signers (keys 1-6).
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

                let v0_blob =
                    celestia_types::Blob::new(*ns, data_a, None).context("v0 blob creation")?;
                blobs_to_submit.push(v0_blob);

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

        while let Some(joined) = join_set.join_next().await {
            joined.context("join failure")??;
        }
    }

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
