use std::sync::atomic::{AtomicUsize, Ordering};

use alloy::{
    network::{Network, ReceiptResponse},
    providers::Provider, rpc::types::TransactionRequest, signers::local::PrivateKeySigner,
};
use arbitrary::Arbitrary;
use alloy::network::TransactionBuilder;
use alloy_primitives::Address;
use alloy_primitives::U256;
use anyhow::Result;
use sov_test_utils::StateWriter;
use std::sync::atomic::AtomicU64;
pub static COMPLETED_TXS: AtomicUsize = AtomicUsize::new(0);
pub static FUNDING_CLIENT_NONCE: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)]
pub async fn run<P, N>(
    client: P,
	funding_client: P,
    tx_count: usize,
    address: Address,
) -> Result<()>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network<TransactionRequest = TransactionRequest> + Send + Sync,
{
    let nonce = client.get_transaction_count(address).await?;
	let chain_id = client.get_chain_id().await?;
	let to_address = Address::arbitrary(&mut arbitrary::Unstructured::new(&[0; 32])).unwrap();
	let balance = client.get_balance(address).await?;
	if balance < tx_count as u128 {
		let tx = TransactionRequest::default()
			.with_to(address)
			.with_value(U256::from((tx_count * 5) as u128))
			.with_max_fee_per_gas(1000) 
			.with_chain_id(chain_id)
			.with_gas_limit(100000)
			.with_max_priority_fee_per_gas(100).with_nonce(FUNDING_CLIENT_NONCE.fetch_add(1, Ordering::SeqCst)); 
		funding_client.send_transaction(tx).await?;
	}


    for i in 0..tx_count {
        let mut wait = std::time::Duration::from_millis(20);
        if i % 100 == 1 {
            println!("TX: {i} of {tx_count} completed");
        }

		let tx = TransactionRequest::default()
			.with_to(to_address)
			.with_value(U256::from(1))
			.with_max_fee_per_gas(1000) 
			.with_chain_id(chain_id)
			.with_gas_limit(100000)
			.with_nonce(nonce + i as u64)
			.with_max_priority_fee_per_gas(100); 
		
        for attempt in 1..=15 {
            match client.send_transaction(tx.clone()).await {
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
