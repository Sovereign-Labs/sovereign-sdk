use std::future::Future;
use std::time::Duration;

use alloy_primitives::{Address, U256};
use sov_eth_client::SimpleStorageClient;

use crate::common::constants::{MAX_POLL_ATTEMPTS, POLL_INTERVAL_MS};

pub async fn poll_until<T, F, Fut, P>(
    mut fetch: F,
    mut predicate: P,
    failure_msg: &str,
) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
    P: FnMut(&T) -> bool,
{
    let mut value = fetch().await?;
    for _ in 0..MAX_POLL_ATTEMPTS {
        if predicate(&value) {
            return Ok(value);
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        value = fetch().await?;
    }
    anyhow::bail!("{failure_msg}")
}

/// Deploys a test contract and asserts the deployed bytecode matches.
pub async fn deploy_contract_check(
    client: &SimpleStorageClient,
) -> Result<Address, Box<dyn std::error::Error>> {
    let runtime_code = client.deploy_contract_call().await?;

    let tx_hash = client.deploy_contract().await?;
    let receipt = client.wait_for_receipt(tx_hash).await;
    let contract_address = receipt.contract_address.unwrap();

    let code = client.eth_get_code(contract_address).await;
    assert_eq!(code[..runtime_code.len()], runtime_code.to_vec());

    Ok(contract_address)
}

/// Calls `set_value` on the test contract and asserts state transitions.
pub async fn set_value_check(
    client: &SimpleStorageClient,
    contract_address: Address,
    set_arg: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let tx_hash = client.set_value(contract_address, set_arg).await;
    client.wait_for_receipt(tx_hash).await;

    let get_arg = client.query_contract(contract_address).await?;
    assert_eq!(U256::from(set_arg), get_arg);

    let storage_slot = 0x0;
    let storage_value = client
        .eth_get_storage_at(contract_address, U256::from(storage_slot))
        .await;
    assert_eq!(storage_value, U256::from(set_arg));

    Ok(())
}

/// Submits multiple `set_values` calls in a single block.
pub async fn set_multiple_values_check(
    client: &SimpleStorageClient,
    contract_address: Address,
    values: Vec<u32>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tx_hashes = client.set_values(contract_address, values).await;
    for tx_hash in tx_hashes {
        client.wait_for_receipt(tx_hash).await;
    }
    let get_arg: u32 = client
        .query_contract(contract_address)
        .await?
        .try_into()
        .unwrap();
    assert!((150..=152).contains(&get_arg));
    Ok(())
}
