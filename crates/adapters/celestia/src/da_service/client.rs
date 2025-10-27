use super::keys::{read_tendermint_key_file, seed_phrase_to_private_key_cosmos};

const QUICK_NODE_RPC_URL: &str = "wss://a-b-c.celestia-mocha.quiknode.pro/grpc_auth_token";
const QUICK_NODE_GRPC_URL: &str = "https://a-b-c.celestia-mochaa.quiknode.pro:9090";
const QUICK_NODE_X_TOKEN: &str = "grpc_auth_token";
const SEED_PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const EXPECTED_SIGNER: &str = "celestia1ht6n9m6j0d8jzq7t0xpsfgd33xzsh4n9f3lvtd";

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn initial_test_quick_node() -> anyhow::Result<()> {
    let private_key_hex = seed_phrase_to_private_key_cosmos(SEED_PHRASE)?;

    let client = celestia_client::Client::builder()
        .rpc_url(QUICK_NODE_RPC_URL)
        .grpc_url(QUICK_NODE_GRPC_URL)
        .grpc_metadata("x-token", QUICK_NODE_X_TOKEN)
        .private_key_hex(&private_key_hex)
        .build()
        .await?;

    let signer = client.address()?;
    assert_eq!(EXPECTED_SIGNER, signer.to_string());

    // HEADER
    let header = client.header().head().await?;
    println!("HEAD HEADER: {header:?}");
    // let namespace_1 = celestia_types::nmt::Namespace::const_v0(*b"sov-mini-i");
    let namespace_2 = celestia_types::nmt::Namespace::const_v0(*b"sov-check-");
    // let height = 8491734;
    // DATA
    // let blobs = client
    //     .blob()
    //     .get_all(height, &[namespace_1])
    //     .await?
    //     .unwrap_or_default();
    // println!("BLOBS: {blobs:?}");
    // let ns_data = client
    //     .share()
    //     .get_namespace_data(height, namespace_1)
    //     .await?;
    // println!("NS DATA: {ns_data:?}");
    // SUBMIT
    let blobx: Vec<u8> = b"hello-sovereign-world".to_vec();

    let blob_completed =
        celestia_types::Blob::new(namespace_2, blobx, None, crate::types::APP_VERSION)?;

    let tx_config = celestia_client::tx::TxConfig::default();

    let result = client.blob().submit(&[blob_completed], tx_config).await?;

    println!("SUBMIT RESULT: {result:?}");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn initial_test_local_docker() -> anyhow::Result<()> {
    let expected_signer_local = "celestia1a68m2l85zn5xh0l07clk4rfvnezhywc53g8x7s";
    let content = include_str!("../../../../../docker/credentials/bridge-0.key");

    // Try with "password" first, then "password\n" if that fails
    let private_key_hex = read_tendermint_key_file(content, "password")?;
    println!("PRIVATE_KEY_HEX: {private_key_hex}");

    let client = celestia_client::Client::builder()
        .rpc_url("ws://127.0.0.1:26658")
        .grpc_url("http://127.0.0.1:9090")
        .private_key_hex(&private_key_hex)
        .build()
        .await?;

    assert_eq!(expected_signer_local, client.address()?.to_string());

    let header = client.header().head().await?;
    println!("HEADER: {header:?}");

    let namespace_2 = celestia_types::nmt::Namespace::const_v0(*b"sov-checkl");
    // let height = 8491734;
    // DATA
    // let blobs = client
    //     .blob()
    //     .get_all(height, &[namespace_1])
    //     .await?
    //     .unwrap_or_default();
    // println!("BLOBS: {blobs:?}");
    // let ns_data = client
    //     .share()
    //     .get_namespace_data(height, namespace_1)
    //     .await?;
    // println!("NS DATA: {ns_data:?}");
    // SUBMIT
    let blobx: Vec<u8> = b"hello-sovereign-world".to_vec();

    let blob_completed =
        celestia_types::Blob::new(namespace_2, blobx, None, crate::types::APP_VERSION)?;

    let tx_config = celestia_client::tx::TxConfig::default();

    let result = client.blob().submit(&[blob_completed], tx_config).await?;

    println!("SUBMIT RESULT: {result:?}");

    Ok(())
}
