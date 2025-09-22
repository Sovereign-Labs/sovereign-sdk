use anyhow::Result;
use ethers::types::TransactionReceipt;
use sov_eth_client::TestClient;
use sov_test_utils::SimpleStorageContract;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

async fn await_and_print_receipt<E>(
    tx_id: u32,
    pending_tx: impl std::future::Future<Output = Result<Option<TransactionReceipt>, E>>,
) where
    E: std::fmt::Debug,
{
    match pending_tx.await {
        Ok(Some(receipt)) => println!(
            "TX {}: Gas: {:?} Block: {:?}",
            tx_id,
            receipt.gas_used.unwrap(),
            receipt.block_number.unwrap()
        ),
        Ok(None) => println!("TX {}: No receipt received", tx_id),
        Err(e) => println!("TX {}: Error - {:?}", tx_id, e),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let rpc_addr: SocketAddr = "127.0.0.1:12346".parse()?;
    let chain_id = 4321;
    let private_key = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let contract = SimpleStorageContract::default();
    let test_client = Arc::new(TestClient::new(chain_id, private_key, contract, rpc_addr).await);

    let deploy_receipt = test_client
        .deploy_contract()
        .await
        .map_err(|e| anyhow::anyhow!("Deploy contract failed: {:?}", e))?
        .await?
        .unwrap();
    let contract_address = deploy_receipt.contract_address.unwrap();

    println!("Contract deployed at: {:?}", contract_address);

    let mut handles = Vec::new();
    for i in 1..=10 {
        let client = Arc::clone(&test_client);
        let handle = tokio::spawn(async move {
            let pending_tx = client.set_value(contract_address, i).await;
            await_and_print_receipt(i, pending_tx).await;
        });
        handles.push(handle);
        // Small delay between sending transactions to avoid overwhelming
        sleep(Duration::from_millis(10)).await;
        if i % 10 == 0 {
            println!("Sent {} transactions...", i);
        }
    }
    println!("All transactions sent, waiting for receipts...");
    for handle in handles {
        let _ = handle.await;
    }
    Ok(())
}
