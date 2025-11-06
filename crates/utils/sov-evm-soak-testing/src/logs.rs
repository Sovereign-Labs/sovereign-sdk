use alloy::eips::BlockNumberOrTag;
use alloy::providers::DynProvider;
use alloy::rpc::types::Filter;
use alloy::signers::local::PrivateKeySigner;
use alloy::{network::Network, providers::Provider};
use alloy_primitives::U256;
use anyhow::Result;
use futures::future::try_join_all;
use sov_eth_client::LogsWithCursorProvider;
use sov_test_utils::SimpleStorage;
use sov_test_utils::Submit;
use std::net::SocketAddr;
use std::time::Instant;

use crate::derive_worker_key;
use crate::{alloy_client, fund_worker_accounts, validate_worker_count};

/// Spawns multiple log test workers, runs them, and retrieves all generated logs.
pub async fn run_logs_test(
    rpc_addr: SocketAddr,
    private_key: &str,
    tx_count: usize,
    logs_per_tx: usize,
    num_workers: usize,
) -> Result<()> {
    validate_worker_count(num_workers)?;
    // Set up root account and fund workers
    let root_signer: PrivateKeySigner = private_key.parse()?;
    let root_client = alloy_client(rpc_addr, root_signer.clone())?;
    fund_worker_accounts(&root_client, &root_signer, private_key, num_workers).await?;
    let from_block = root_client.get_block_number().await?;

    produce_logs(rpc_addr, private_key, num_workers, tx_count, logs_per_tx).await?;
    retrieve_logs(root_client, from_block).await?;

    Ok(())
}

async fn produce_logs(
    rpc_addr: SocketAddr,
    private_key: &str,
    num_workers: usize,
    tx_count: usize,
    logs_per_tx: usize,
) -> Result<()> {
    let timer = Instant::now();
    let mut handles = Vec::with_capacity(num_workers);
    for worker_idx in 0..num_workers {
        let signer: PrivateKeySigner = derive_worker_key(private_key, worker_idx)?.parse()?;
        let client = alloy_client(rpc_addr, signer.clone())?;

        handles.push(tokio::spawn(async move {
            match LogsSoakTest::new(client, worker_idx).await {
                Ok(test) => {
                    if let Err(e) = test.run(tx_count, logs_per_tx).await {
                        eprintln!("Worker {worker_idx} error during run: {e:?}");
                    }
                }
                Err(e) => {
                    eprintln!("Worker {worker_idx} failed to deploy contracts: {e:?}");
                }
            }
            Ok::<(), anyhow::Error>(())
        }));
    }
    try_join_all(handles).await?;
    println!(
        "Produced {} logs in {:?}",
        num_workers * tx_count * logs_per_tx,
        timer.elapsed()
    );
    Ok(())
}

async fn retrieve_logs(client: DynProvider, from_block: u64) -> Result<()> {
    let filter: Filter = Filter::new()
        .from_block(from_block)
        .to_block(BlockNumberOrTag::Pending);
    let timer = Instant::now();
    let logs = client.get_all_logs_with_cursor(&filter).await?;
    println!(
        "Retrieved {} logs in {:?}",
        logs.len(),
        timer.elapsed()
    );
    Ok(())
}

pub struct LogsSoakTest<P, N> {
    contract: SimpleStorage::SimpleStorageInstance<P, N>,
    idx: usize,
}

impl<P, N> LogsSoakTest<P, N>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    pub async fn new(client: P, idx: usize) -> Result<Self> {
        let address = SimpleStorage::deploy_builder(client.clone())
            .gas(30_000_000)
            .deploy()
            .await?;
        let contract = SimpleStorage::new(address, client);
        Ok(Self { contract, idx })
    }

    pub async fn run(self, tx_count: usize, logs_per_tx: usize) -> Result<()> {
        for i in 1..=tx_count {
            println!("{}: Sending tx {i} with {logs_per_tx} logs", self.idx);
            self.contract
                .emitLogs(U256::ZERO, U256::from(logs_per_tx))
                .submit()
                .await?;
        }
        Ok(())
    }
}
