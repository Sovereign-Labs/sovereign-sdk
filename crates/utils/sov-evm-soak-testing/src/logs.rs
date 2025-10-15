use alloy::{network::Network, providers::Provider};
use alloy_primitives::U256;
use anyhow::Result;
use sov_test_utils::SimpleStorage;
use sov_test_utils::Submit;

pub struct LogsSoakTest<P, N> {
    contract: SimpleStorage::SimpleStorageInstance<P, N>,
}

impl<P, N> LogsSoakTest<P, N>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    pub async fn new(client: P) -> Result<Self> {
        let contract = SimpleStorage::deploy(client).await?;
        Ok(Self { contract })
    }

    pub async fn run(self, tx_count: usize, logs_per_tx: usize) -> Result<()> {
        for i in 1..=tx_count {
            println!("Sending tx {} with {} logs", i, logs_per_tx);
            self.contract
                .emitLogs(U256::ZERO, U256::from(logs_per_tx))
                .submit()
                .await?;
        }
        Ok(())
    }
}
