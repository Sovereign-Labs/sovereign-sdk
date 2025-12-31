use alloy::{
    network::{Network, ReceiptResponse},
    providers::Provider,
};
use alloy_primitives::U256;
use anyhow::Result;
use sov_evm_test_utils::SimpleStorage;

#[allow(dead_code)]
pub async fn run<P, N>(client: P) -> Result<()>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    let contract = SimpleStorage::deploy(client).await?;

    println!("Contract deployed at: {:?}", contract.address());

    for i in 1..=1000 {
        let tx = contract.set(U256::from(i)).send().await?;
        let receipt = tx.get_receipt().await?;
        println!(
            "TX: {} Gas: {:?} Block: {:?}",
            i,
            receipt.gas_used(),
            receipt.block_number()
        );
    }
    Ok(())
}
