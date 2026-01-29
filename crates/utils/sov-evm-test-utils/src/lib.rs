use alloy::{contract::SolCallBuilder, network::Network, providers::Provider, sol_types::SolCall};
use anyhow::Result;

mod block_hash;
mod fake_uni;
mod simple_storage;
pub use block_hash::BlockHash;
pub use fake_uni::{Erc20, Router};
pub use simple_storage::{LegacySimpleStorage, SimpleStorage};

/// Helper trait to submit contract calls without needing to handle the response
#[async_trait::async_trait]
pub trait Submit {
    /// Submit the contract call transaction and wait for confirmation
    async fn submit(self) -> Result<()>;
}

#[async_trait::async_trait]
#[allow(clippy::extra_unused_lifetimes)]
impl<'a, P, C, N> Submit for SolCallBuilder<P, C, N>
where
    P: Provider<N> + Clone + Send + Sync + 'a,
    C: SolCall + Send + Sync,
    N: Network + Send + Sync + 'a,
{
    async fn submit(self) -> Result<()> {
        let _ = self.send().await?;
        Ok(())
    }
}
