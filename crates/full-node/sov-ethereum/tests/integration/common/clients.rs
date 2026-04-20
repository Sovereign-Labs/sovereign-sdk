use std::net::SocketAddr;

use alloy::signers::local::PrivateKeySigner;
use alloy_provider::{DynProvider, Provider, ProviderBuilder, WsConnect};
use reqwest::Url;
use sov_eth_client::SimpleStorageClient;
use sov_evm_test_utils::LegacySimpleStorage;

use crate::common::constants::SENDER_PRIV_KEY;

pub async fn create_simple_storage_client(
    rest_port: SocketAddr,
    private_key: &str,
) -> SimpleStorageClient {
    let contract = LegacySimpleStorage::default();
    SimpleStorageClient::new(private_key, contract, rest_port).await
}

pub async fn alloy_ws_client(socket: SocketAddr) -> DynProvider {
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse().unwrap();
    let url = Url::parse(&format!("ws://{socket}/rpc")).unwrap();
    let ws = WsConnect::new(url);
    ProviderBuilder::new()
        .wallet(signer)
        .connect_ws(ws)
        .await
        .unwrap()
        .erased()
}

pub fn alloy_client_with_signer(socket: SocketAddr, private_key: &str) -> DynProvider {
    let signer: PrivateKeySigner = private_key.parse().unwrap();
    let url = Url::parse(&format!("http://{socket}/rpc")).unwrap();
    ProviderBuilder::new()
        .wallet(signer)
        .connect_http(url)
        .erased()
}

pub fn alloy_client(socket: SocketAddr) -> DynProvider {
    alloy_client_with_signer(socket, SENDER_PRIV_KEY)
}

pub fn alloy_client_with_reqwest<B>(socket: SocketAddr, b: B, private_key: &str) -> DynProvider
where
    B: FnOnce(reqwest::ClientBuilder) -> reqwest::Client,
{
    let signer: PrivateKeySigner = private_key.parse().unwrap();
    let url = Url::parse(&format!("http://{socket}/rpc")).unwrap();
    ProviderBuilder::new()
        .wallet(signer)
        .with_reqwest(url, b)
        .erased()
}
