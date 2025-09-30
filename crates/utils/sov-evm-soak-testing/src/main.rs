use anyhow::Result;
use sov_eth_client::TestClient;
use sov_test_utils::SimpleStorage;
use std::net::SocketAddr;

mod simple_storage;
mod uniswap;

#[tokio::main]
async fn main() -> Result<()> {
    let rpc_addr: SocketAddr = "127.0.0.1:12346".parse()?;
    let private_key = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let contract = SimpleStorage::default();
    let client = TestClient::new(private_key, contract, rpc_addr).await;

    // Run uniswap soak test
    uniswap::run(client).await?;

    // If you want to run simple storage test instead:
    // simple_storage::run(client).await?;

    Ok(())
}
