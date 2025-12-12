use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, Bytes, TxHash, U256};
use alloy_provider::DynProvider;
use alloy_provider::Provider as _;
use alloy_provider::ProviderBuilder;
use alloy_pubsub::Subscription;
use alloy_rpc_types::{Block, Filter, Log, Transaction, TransactionReceipt, TransactionRequest};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use jsonrpsee::ws_client::WsClientBuilder;
use sov_rpc_eth_types::FilterWithCursor;
use sov_rpc_eth_types::LogWithExecutionTimestamp;
use sov_rpc_eth_types::LogsWithMaybeCursor;

pub struct RpcClient {
    pub ws: jsonrpsee::ws_client::WsClient,
    pub pub_sub: alloy_provider::RootProvider,
    pub alloy_client: DynProvider,
    signer: PrivateKeySigner,
    chain_id: u64,
}

impl RpcClient {
    pub async fn new(private_key: &str, http_addr: std::net::SocketAddr) -> Self {
        let http_conn_str = &format!("http://127.0.0.1:{}/rpc", http_addr.port());
        let ws_conn_str = &format!("ws://127.0.0.1:{}/rpc", http_addr.port());

        let ws = WsClientBuilder::default().build(ws_conn_str).await.unwrap();

        let pub_sub = ProviderBuilder::default()
            .connect(ws_conn_str)
            .await
            .unwrap();

        let signer: PrivateKeySigner = private_key.parse().unwrap();
        let alloy_client = ProviderBuilder::new()
            .wallet(signer.clone())
            .connect_http(http_conn_str.parse().unwrap())
            .erased();

        let chain_id = alloy_client.get_chain_id().await.unwrap();

        Self {
            alloy_client,
            ws,
            pub_sub,
            signer,
            chain_id,
        }
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn address(&self) -> Address {
        self.signer.address()
    }
}

// Alloy client methods
impl RpcClient {
    pub async fn eth_accounts(&self) -> Vec<Address> {
        self.alloy_client.get_accounts().await.unwrap()
    }

    pub async fn receipt(&self, hash: TxHash) -> Option<TransactionReceipt> {
        self.alloy_client
            .get_transaction_receipt(hash)
            .await
            .unwrap()
    }

    pub async fn transaction(&self, hash: TxHash) -> Option<Transaction> {
        self.alloy_client
            .get_transaction_by_hash(hash)
            .await
            .unwrap()
    }

    pub async fn eth_chain_id(&self) -> u64 {
        self.alloy_client.get_chain_id().await.unwrap()
    }

    pub async fn eth_get_balance(&self, address: Address) -> U256 {
        self.alloy_client.get_balance(address).await.unwrap()
    }

    pub async fn eth_get_code(&self, address: Address) -> Vec<u8> {
        self.alloy_client
            .get_code_at(address)
            .await
            .unwrap()
            .to_vec()
    }

    pub async fn eth_get_transaction_count(&self, address: Address) -> u64 {
        self.alloy_client
            .get_transaction_count(address)
            .await
            .unwrap()
    }

    pub async fn eth_estimate_gas(&self, tx: TransactionRequest) -> u64 {
        self.alloy_client.estimate_gas(tx).await.unwrap()
    }

    pub async fn eth_send_transaction(
        &self,
        tx: TransactionRequest,
    ) -> Result<TxHash, Box<dyn std::error::Error>> {
        let pending = self.alloy_client.send_transaction(tx).await?;
        Ok(*pending.tx_hash())
    }

    pub async fn block_number(&self) -> u64 {
        self.alloy_client.get_block_number().await.unwrap()
    }

    pub async fn eth_gas_price(&self) -> u128 {
        self.alloy_client.get_gas_price().await.unwrap()
    }
}

// Jsonrpsee WS client
impl RpcClient {
    pub async fn eth_get_block_by_number(&self, block_number: Option<String>) -> Block {
        self.ws
            .request("eth_getBlockByNumber", rpc_params![block_number, false])
            .await
            .unwrap()
    }

    pub async fn eth_call(
        &self,
        tx: TransactionRequest,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        self.ws
            .request("eth_call", rpc_params![tx])
            .await
            .map_err(|e| e.into())
    }

    pub async fn eth_get_storage_at(&self, address: Address, index: U256) -> U256 {
        self.ws
            .request("eth_getStorageAt", rpc_params![address, index])
            .await
            .unwrap()
    }

    pub async fn alloy_get_block_by_number(&self, block_number: Option<String>) -> Block {
        self.ws
            .request("eth_getBlockByNumber", rpc_params![block_number, false])
            .await
            .unwrap()
    }

    pub async fn get_logs_allow_error(&self) -> Result<Vec<Log>, Box<dyn std::error::Error>> {
        self.ws
            .request(
                "eth_getLogs",
                rpc_params![serde_json::json!({
                    "fromBlock": "0x0",
                    "toBlock": "latest",
                })],
            )
            .await
            .map_err(|e| e.into())
    }

    pub async fn get_logs_with_cursor(&self, filter: &FilterWithCursor) -> LogsWithMaybeCursor {
        self.ws
            .request("eth_getLogsWithCursor", rpc_params![filter])
            .await
            .unwrap()
    }

    pub async fn get_logs_with_cursor_and_filter(
        &self,
        cursor_and_filter: &impl serde::Serialize,
    ) -> LogsWithMaybeCursor {
        self.ws
            .request("eth_getLogsWithCursor", rpc_params![cursor_and_filter])
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

    pub async fn alloy_transaction(&self, hash: TxHash) -> Option<Transaction> {
        self.pub_sub.get_transaction_by_hash(hash).await.unwrap()
    }

    pub async fn alloy_receipt(&self, hash: TxHash) -> Option<TransactionReceipt> {
        self.pub_sub.get_transaction_receipt(hash).await.unwrap()
    }

    pub async fn get_logs(&self, filter: &Filter) -> Vec<Log> {
        self.pub_sub.get_logs(filter).await.unwrap()
    }

    pub async fn get_logs_with_timestamp(&self, filter: &Filter) -> Vec<LogWithExecutionTimestamp> {
        self.pub_sub
            .client()
            .request("eth_getLogs", (filter,))
            .await
            .unwrap()
    }
}
