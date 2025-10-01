use anyhow::Result;
use sov_eth_client::SimpleStorageClient;

#[allow(dead_code)]
pub async fn run(client: SimpleStorageClient) -> Result<()> {
    let deploy_receipt = client
        .deploy_contract()
        .await
        .map_err(|e| anyhow::anyhow!("Deploy contract failed: {:?}", e))?
        .await?
        .unwrap();
    let contract_address = deploy_receipt.contract_address.unwrap();

    println!("Contract deployed at: {contract_address:?}");

    for i in 1..=1000 {
        let pending_tx = client.set_value(contract_address, i).await;
        match pending_tx.await {
            Ok(Some(receipt)) => println!(
                "TX {}: Gas: {:?} Block: {:?}",
                i,
                receipt.gas_used.unwrap(),
                receipt.block_number.unwrap()
            ),
            Ok(None) => println!("TX {i}: No receipt received"),
            Err(e) => println!("TX {i}: Error - {e:?}"),
        }
    }
    Ok(())
}
