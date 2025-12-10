use crate::evm::evm_test_helper::setup_with_simple_storage;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::test_helpers::DemoRollupSpec;
use ethers::core::abi::Address;
use ethers::signers::Signer;
use sov_address::{EthereumAddress, MultiAddress};
use sov_bank::config_gas_token_id;
use sov_demo_rollup::MockDemoRollup;
use sov_modules_api::execution_mode::Native;
use sov_test_utils::test_rollup::{self};
use std::str::FromStr;

const RECIEVER_ADDR_STR: &str = "0x3FE0233e6cf3c9753fcB7449987EC49C88aDDE71";

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_foo() -> anyhow::Result<()> {
    use jsonrpsee::core::client::ClientT;
    use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
    use reqwest::header::HeaderMap;
    use reqwest::header::HeaderValue;

    let (test_rollup, evm_client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let rpc_url = test_rollup.http_addr.to_string();
    let rpc_url = format!("http://{}/rpc", rpc_url);

    let reciever_address = Address::from_str(RECIEVER_ADDR_STR).unwrap();
    let mut tx = evm_client.make_tx(Some(reciever_address), None);
    tx.set_value(2);

    //self.client.sign_transaction(rx);
    let signature = evm_client
        .client
        .signer()
        .sign_transaction(&tx)
        .await
        .unwrap();

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("123.123.123.123"),
    );

    let client = HttpClientBuilder::default()
        .set_headers(headers)
        .build(rpc_url)?;

    let tx: ethers::types::Bytes = tx.rlp_signed(&signature);
    let raw_hex = format!("0x{}", hex::encode(tx));

    let tx_hash: String = client
        .request("eth_sendRawTransaction", vec![raw_hex])
        .await
        .unwrap();

    println!("tx hash: {}", tx_hash);
    Ok(())
}
