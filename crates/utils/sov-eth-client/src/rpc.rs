use alloy_primitives::Bytes;
use alloy_provider::Provider as _;
use alloy_provider::ProviderBuilder;
use alloy_pubsub::Subscription;
use alloy_rpc_types::Filter;
use alloy_rpc_types::Log;
use ethers::core::abi::Address;
use ethers::core::k256::ecdsa::SigningKey;
use ethers::core::types::transaction::eip2718::TypedTransaction;
use ethers::core::types::{Block, TxHash};
use ethers::core::types::{Transaction, TransactionReceipt};
use ethers::middleware::SignerMiddleware;
use ethers::providers::{Http, Middleware, PendingTransaction, Provider};
use ethers::signers::Wallet;
use ethers::signers::{LocalWallet, Signer};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use jsonrpsee::ws_client::WsClientBuilder;

pub struct RpcClient {
    pub client: ethers::middleware::SignerMiddleware<Provider<Http>, Wallet<SigningKey>>,
    pub ws: jsonrpsee::ws_client::WsClient,
    pub pub_sub: alloy_provider::RootProvider,
}

impl RpcClient {
    pub async fn new(private_key: &str, http_addr: std::net::SocketAddr) -> Self {
        let http_conn_str = &format!("http://127.0.0.1:{}/rpc", http_addr.port());
        let ws_conn_str = &format!("ws://127.0.0.1:{}/rpc", http_addr.port());
        let wallet = private_key.parse::<LocalWallet>().unwrap();
        let provider = Provider::try_from(http_conn_str).unwrap();
        let client = SignerMiddleware::new_with_provider_chain(provider, wallet)
            .await
            .unwrap();
        let ws = WsClientBuilder::default().build(ws_conn_str).await.unwrap();
        let pub_sub = ProviderBuilder::default()
            .connect(&ws_conn_str)
            .await
            .unwrap();

        Self {
            client,
            ws,
            pub_sub,
        }
    }

    pub fn chain_id(&self) -> u64 {
        self.client.signer().chain_id()
    }
    pub fn address(&self) -> Address {
        self.client.signer().address()
    }
}

// Ethers client
impl RpcClient {
    pub async fn eth_accounts(&self) -> Vec<Address> {
        self.client.get_accounts().await.unwrap()
    }

    pub async fn receipt(&self, hash: TxHash) -> Option<TransactionReceipt> {
        self.client.get_transaction_receipt(hash).await.unwrap()
    }

    pub async fn transaction(&self, hash: TxHash) -> Option<Transaction> {
        self.client.get_transaction(hash).await.unwrap()
    }

    pub async fn eth_chain_id(&self) -> u64 {
        self.client.get_chainid().await.unwrap().as_u64()
    }

    pub async fn eth_get_balance(&self, address: Address) -> ethereum_types::U256 {
        self.client.get_balance(address, None).await.unwrap()
    }

    pub async fn eth_get_code(&self, address: Address) -> Vec<u8> {
        self.client.get_code(address, None).await.unwrap().to_vec()
    }

    pub async fn eth_get_transaction_count(&self, address: Address) -> u64 {
        self.client
            .get_transaction_count(address, None)
            .await
            .unwrap()
            .as_u64()
    }

    pub async fn eth_estimate_gas(&self, tx: TypedTransaction) -> u64 {
        self.client.estimate_gas(&tx, None).await.unwrap().as_u64()
    }

    pub async fn eth_send_transaction(
        &self,
        tx: TypedTransaction,
    ) -> Result<PendingTransaction<'_, Http>, Box<dyn std::error::Error>> {
        self.client
            .send_transaction(tx, None)
            .await
            .map_err(Into::into)
    }

    pub async fn block_number(&self) -> u64 {
        self.client.get_block_number().await.unwrap().as_u64()
    }

    pub async fn eth_gas_price(&self) -> u128 {
        self.client.get_gas_price().await.unwrap().as_u128()
    }
}

// Jsonrpsee WS client
impl RpcClient {
    pub async fn eth_get_block_by_number(&self, block_number: Option<String>) -> Block<TxHash> {
        self.ws
            .request("eth_getBlockByNumber", rpc_params![block_number, false])
            .await
            .unwrap()
    }

    pub async fn eth_call(
        &self,
        tx: TypedTransaction,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        self.ws
            .request("eth_call", rpc_params![tx])
            .await
            .map_err(|e| e.into())
    }

    pub async fn eth_get_storage_at(
        &self,
        address: Address,
        index: ethereum_types::U256,
    ) -> ethereum_types::U256 {
        self.ws
            .request("eth_getStorageAt", rpc_params![address, index])
            .await
            .unwrap()
    }

    pub async fn alloy_get_block_by_number(
        &self,
        block_number: Option<String>,
    ) -> alloy_rpc_types::Block<alloy_primitives::TxHash> {
        self.ws
            .request("eth_getBlockByNumber", rpc_params![block_number, false])
            .await
            .unwrap()
    }
}

// Alloy pubsub client
impl RpcClient {
    pub async fn alloy_subscribe_logs(&self, filter: &Filter) -> Subscription<Log> {
        self.pub_sub.subscribe_logs(filter).await.unwrap()
    }

    pub fn alloy_unsubscribe(&self, id: alloy_primitives::B256) {
        self.pub_sub.unsubscribe(id).unwrap();
    }

    pub async fn alloy_transaction(
        &self,
        hash: alloy_primitives::TxHash,
    ) -> Option<alloy_rpc_types::Transaction> {
        self.pub_sub.get_transaction_by_hash(hash).await.unwrap()
    }

    pub async fn alloy_receipt(
        &self,
        hash: alloy_primitives::TxHash,
    ) -> Option<alloy_rpc_types::TransactionReceipt> {
        self.pub_sub.get_transaction_receipt(hash).await.unwrap()
    }

    pub async fn get_logs(&self, filter: &Filter) -> Vec<Log> {
        self.pub_sub.get_logs(filter).await.unwrap()
    }
}
