use crate::GAS;
use alloy::signers::local::PrivateKeySigner;
use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_eips::Encodable2718;
use alloy_primitives::{Address, Bytes, TxHash, TxKind, B256, U256};
use alloy_provider::Provider as _;
use alloy_provider::ProviderBuilder;
use alloy_provider::RootProvider;
use alloy_pubsub::Subscription;
use alloy_rpc_types::{Block, Filter, Log, Transaction, TransactionReceipt, TransactionRequest};
use alloy_signer::SignerSync;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use jsonrpsee::ws_client::WsClientBuilder;
use sov_rpc_eth_types::FilterWithCursor;
use sov_rpc_eth_types::LogWithExecutionTimestamp;
use sov_rpc_eth_types::LogsWithMaybeCursor;

pub struct RpcClient {
    pub ws: jsonrpsee::ws_client::WsClient,
    pub pub_sub: RootProvider,
    http_provider: RootProvider,
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

        // Use a simple provider without wallet filler to avoid the `to` requirement
        let http_provider = ProviderBuilder::default().connect_http(http_conn_str.parse().unwrap());

        let chain_id = http_provider.get_chain_id().await.unwrap();

        Self {
            http_provider,
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
        self.http_provider.get_accounts().await.unwrap()
    }

    pub async fn receipt(&self, hash: TxHash) -> Option<TransactionReceipt> {
        self.http_provider
            .get_transaction_receipt(hash)
            .await
            .unwrap()
    }

    pub async fn transaction(&self, hash: TxHash) -> Option<Transaction> {
        self.http_provider
            .get_transaction_by_hash(hash)
            .await
            .unwrap()
    }

    pub async fn eth_chain_id(&self) -> u64 {
        self.http_provider.get_chain_id().await.unwrap()
    }

    pub async fn eth_get_balance(&self, address: Address) -> U256 {
        self.http_provider.get_balance(address).await.unwrap()
    }

    pub async fn eth_get_code(&self, address: Address) -> Vec<u8> {
        self.http_provider
            .get_code_at(address)
            .await
            .unwrap()
            .to_vec()
    }

    pub async fn eth_get_transaction_count(&self, address: Address) -> u64 {
        self.http_provider
            .get_transaction_count(address)
            .await
            .unwrap()
    }

    pub async fn eth_estimate_gas(&self, tx: TransactionRequest) -> u64 {
        self.http_provider.estimate_gas(tx).await.unwrap()
    }

    pub async fn eth_send_transaction(
        &self,
        tx: TransactionRequest,
    ) -> Result<TxHash, Box<dyn std::error::Error>> {
        // Build an EIP-1559 transaction from the request
        let eip1559_tx = TxEip1559 {
            chain_id: self.chain_id,
            nonce: tx.nonce.unwrap_or(0),
            gas_limit: tx.gas.unwrap_or(GAS),
            max_fee_per_gas: tx.max_fee_per_gas.unwrap_or(0),
            max_priority_fee_per_gas: tx.max_priority_fee_per_gas.unwrap_or(0),
            to: tx.to.unwrap_or(TxKind::Create),
            value: tx.value.unwrap_or(U256::ZERO),
            input: tx.input.into_input().unwrap_or_default(),
            access_list: Default::default(),
        };

        // Sign the transaction
        let sig = self.signer.sign_hash_sync(&eip1559_tx.signature_hash())?;
        let signed = eip1559_tx.into_signed(sig);

        // Wrap in TxEnvelope for proper EIP-2718 encoding
        let envelope = TxEnvelope::Eip1559(signed);
        let encoded = envelope.encoded_2718();

        let pending = self.http_provider.send_raw_transaction(&encoded).await?;
        Ok(*pending.tx_hash())
    }

    pub async fn block_number(&self) -> u64 {
        self.http_provider.get_block_number().await.unwrap()
    }

    pub async fn eth_gas_price(&self) -> u128 {
        self.http_provider.get_gas_price().await.unwrap()
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
        let value: B256 = self
            .ws
            .request("eth_getStorageAt", rpc_params![address, index])
            .await
            .unwrap();
        U256::from_be_slice(value.as_slice())
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
