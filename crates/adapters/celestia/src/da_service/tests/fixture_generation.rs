use super::*;

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
