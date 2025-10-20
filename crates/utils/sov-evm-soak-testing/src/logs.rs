use alloy::{network::Network, providers::Provider};
use alloy_primitives::U256;
use anyhow::Result;
use sov_test_utils::SimpleStorage;
use sov_test_utils::Submit;

/// Spawns multiple log test workers, runs them, and retrieves all generated logs.
async fn run_logs_test(
    rpc_addr: SocketAddr,
    private_key: &str,
    tx_count: usize,
    logs_per_tx: usize,
    num_workers: usize,
) -> Result<()> {
    validate_worker_count(num_workers)?;

    // Set up root account and fund workers
    let root_signer: PrivateKeySigner = private_key.parse()?;
    let root_client = alloy_client(rpc_addr, root_signer.clone()).await?;

    fund_worker_accounts(&root_client, &root_signer, private_key, num_workers).await?;

    let from_block = root_client.get_block_number().await?;

    // Spawn workers
    let mut handles = Vec::with_capacity(num_workers);
    for worker_idx in 0..num_workers {
        let signer: PrivateKeySigner = derive_worker_key(private_key, worker_idx)?.parse()?;
        let client = alloy_client(rpc_addr, signer.clone()).await?;

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

    // Retrieve and count all logs
    let to_block = root_client.get_block_number().await?;
    let filter = Filter::new().from_block(from_block).to_block(to_block);
    let logs = root_client.get_logs(&filter).await?;

    println!("Total logs retrieved: {}", logs.len());

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
