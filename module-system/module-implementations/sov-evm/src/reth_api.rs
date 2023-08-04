use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;

use reth_beacon_consensus::{BeaconConsensus, BeaconConsensusEngine, MIN_BLOCKS_FOR_PIPELINE_RUN};
use reth_blockchain_tree::config::BlockchainTreeConfig;
use reth_blockchain_tree::externals::TreeExternals;
use reth_blockchain_tree::{BlockchainTree, ShareableBlockchainTree};
use reth_interfaces::blockchain_tree::BlockchainTreeEngine;
use reth_network_api::{NetworkError, NetworkInfo};
use reth_primitives::{TxHash, MAINNET};
use reth_provider::providers::BlockchainProvider;
use reth_provider::{BlockReaderIdExt, EvmEnvProvider, ProviderFactory, StateProviderFactory};
use reth_revm::Factory;
use reth_rpc::eth::cache::{EthStateCache, EthStateCacheConfig};
use reth_rpc::eth::EthTransactions;
use reth_rpc::EthApi;
use reth_rpc_types::NetworkStatus;
use reth_tasks::TokioTaskExecutor;
use reth_transaction_pool::validate::EthTransactionValidatorBuilder;
use reth_transaction_pool::{
    AllPoolTransactions, AllTransactionsEvents, BestTransactions, BlockInfo, CoinbaseTipOrdering,
    EthTransactionValidator, Pool, PoolResult, PoolSize, PropagatedTransactions, TransactionEvents,
    TransactionOrdering, TransactionOrigin, TransactionPool, ValidPoolTransaction,
};
use revm::primitives::Address;
use sov_db::reth_db::DatabaseMock;

struct MockNetwork {}

impl NetworkInfo for MockNetwork {
    fn local_addr(&self) -> SocketAddr {
        todo!()
    }

    fn network_status<'life0, 'async_trait>(
        &'life0 self,
    ) -> ::core::pin::Pin<
        Box<
            dyn ::core::future::Future<Output = Result<NetworkStatus, NetworkError>>
                + ::core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        todo!()
    }

    fn chain_id(&self) -> u64 {
        todo!()
    }

    fn is_syncing(&self) -> bool {
        todo!()
    }
}

struct Api<Provider, Pool, Network> {
    api: EthApi<Provider, Pool, Network>,
}

impl<Provider, P, Network> Api<Provider, P, Network>
where
    P: TransactionPool + Clone + 'static,
    Provider: BlockReaderIdExt + StateProviderFactory + EvmEnvProvider + 'static,
    Network: NetworkInfo + Send + Sync + 'static,
{
    fn new() -> Self {
        let gas_cap: u64 = 0;

        let network = MockNetwork {};
        let database = DatabaseMock { data: todo!() };

        let c = EthStateCacheConfig::default();

        let gas_oracle = todo!();
        let tracing_call_pool = todo!();

        /// Blokchain Provider
        let chain_spec = todo!();
        let provider_factory = ProviderFactory::new(database, chain_spec);

        let consensus = Arc::new(BeaconConsensus::new(Arc::clone(&chain_spec)));
        // configure blockchain tree
        let tree_externals = TreeExternals::new(
            database.clone(),
            Arc::clone(&consensus),
            Factory::new(chain_spec.clone()),
            Arc::clone(&chain_spec),
        );
        let tree_config = BlockchainTreeConfig::default();

        let (canon_state_notification_sender, _receiver) =
            tokio::sync::broadcast::channel(tree_config.max_reorg_depth() as usize * 2);

        let blockchain_tree = ShareableBlockchainTree::new(
            BlockchainTree::new(
                tree_externals,
                canon_state_notification_sender.clone(),
                tree_config,
            )
            .unwrap(),
        );

        let provider = BlockchainProvider::new(provider_factory, blockchain_tree).unwrap();

        let eth_cache = EthStateCache::spawn(provider, c);

        let database = DatabaseMock { data: todo!() };
        ///Pool
        let pool = Pool::eth_pool(
            EthTransactionValidator::new(provider, MAINNET.clone(), TokioTaskExecutor::default()),
            Default::default(),
        );

        EthApi::new(
            provider,
            pool,
            network,
            eth_cache,
            gas_oracle,
            gas_cap,
            tracing_call_pool,
        );
        todo!()
    }

    fn foo(&self) {
        let h = todo!();
        self.api.transaction_by_hash(h);
    }
}
