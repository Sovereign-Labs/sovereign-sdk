mod test_helpers;
use std::net::SocketAddr;

use borsh::BorshSerialize;
use demo_stf::app::DefaultPrivateKey;
use demo_stf::runtime::RuntimeCall;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{PrivateKey, Spec};
use test_helpers::start_rollup;

struct TestClient {
    client: HttpClient,
}

impl TestClient {
    pub fn new(endpoint: &str) -> Self {
        let client = HttpClientBuilder::default().build(endpoint).unwrap();
        Self { client }
    }
    pub async fn send_transaction(
        &self,
        tx: Transaction<DefaultContext>,
    ) -> Result<(), anyhow::Error> {
        let batch = vec![tx.try_to_vec()?];
        let response: String = self.client.request("sequencer_publishBatch", batch).await?;
        println!("response: {:?}", response);
        Ok(())
    }
}

async fn send_test_create_token_tx(rpc_address: SocketAddr) -> Result<(), anyhow::Error> {
    let key = DefaultPrivateKey::generate();
    let address: <DefaultContext as Spec>::Address = key.to_address();

    let msg = RuntimeCall::bank(sov_bank::CallMessage::<DefaultContext>::CreateToken {
        salt: 0,
        token_name: "test-token".to_string(),
        initial_balance: 1000,
        minter_address: address,
        authorized_minters: vec![],
    });
    let tx = Transaction::<DefaultContext>::new_signed_tx(&key, msg.try_to_vec().unwrap(), 0);

    let client = TestClient::new(&format!("http://localhost:{}", rpc_address.port()));

    client.send_transaction(tx).await
}

#[tokio::test]
async fn tx_tests() -> Result<(), anyhow::Error> {
    let (port_tx, port_rx) = tokio::sync::oneshot::channel();

    let rollup_task = tokio::spawn(async {
        start_rollup(port_tx).await;
    });

    // Wait for rollup task to start:
    let port = port_rx.await.unwrap();

    send_test_create_token_tx(port).await?;
    rollup_task.abort();
    Ok(())
}
