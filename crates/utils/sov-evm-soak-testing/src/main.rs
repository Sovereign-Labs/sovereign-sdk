use anyhow::Result;
use sov_eth_client::TestClient;
use sov_test_utils::SimpleStorageContract;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<()> {
    let rpc_addr: SocketAddr = "127.0.0.1:12346".parse()?;
    let private_key = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let contract = SimpleStorageContract::default();
    let client = TestClient::new(private_key, contract, rpc_addr).await;

    let deploy_receipt = client
        .deploy_contract()
        .await
        .map_err(|e| anyhow::anyhow!("Deploy contract failed: {:?}", e))?
        .await?
        .unwrap();
    let contract_address = deploy_receipt.contract_address.unwrap();

    println!("Contract deployed at: {:?}", contract_address);

    for i in 1..=1000 {
        let pending_tx = client.set_value(contract_address, i).await;
        match pending_tx.await {
            Ok(Some(receipt)) => println!(
                "TX {}: Gas: {:?} Block: {:?}",
                i,
                receipt.gas_used.unwrap(),
                receipt.block_number.unwrap()
            ),
            Ok(None) => println!("TX {}: No receipt received", i),
            Err(e) => println!("TX {}: Error - {:?}", i, e),
        }
    }
    Ok(())
}
