use std::sync::atomic::{AtomicUsize, Ordering};

use alloy::{
    network::{Network, ReceiptResponse},
    providers::Provider,
};
use alloy_primitives::Address;
use alloy_primitives::U256;
use anyhow::Result;
use sov_test_utils::StateWriter;
pub static COMPLETED_TXS: AtomicUsize = AtomicUsize::new(0);

#[allow(dead_code)]
pub async fn run<P, N>(
    client: P,
    tx_count: usize,
    writes_per_tx: usize,
    overlapping_writes_per_tx: usize,
    address: Address,
) -> Result<()>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    let nonce = client.get_transaction_count(address).await?;
    let contract = StateWriter::deploy(client).await?;

    println!("Contract deployed at: {:?}", contract.address());

    for i in 1..=tx_count {
        let mut wait = std::time::Duration::from_millis(20);
        if i % 100 == 1 {
            println!("TX: {i} of {tx_count} completed");
        }
        let starting_point = i
            * (writes_per_tx
                .checked_sub(overlapping_writes_per_tx)
                .expect("overlapping_writes_per_tx must be no greater than writes_per_tx"));
        for attempt in 1..=15 {
            match contract
                .testWriteValuesAt(
                    U256::from(starting_point),
                    U256::from(writes_per_tx),
                    U256::from(overlapping_writes_per_tx),
                )
                .nonce(nonce + i as u64)
                .send()
                .await
            {
                Ok(tx) => {
                    break;
                }
                Err(e) => {
                    if attempt == 15 {
                        println!("Error sending TX {i}: {e:?}");
                        return Err(e.into());
                    }
                }
            }
            tokio::time::sleep(wait).await;
            wait *= 2;
            wait = std::cmp::min(wait, std::time::Duration::from_secs(1));
        }
        let previous_completed = COMPLETED_TXS.fetch_add(1, Ordering::Relaxed);
    }
    println!("All {tx_count} TXs completed");
    Ok(())
}
