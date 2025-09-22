#![allow(missing_docs)]

use alloy_primitives::Bytes;
use alloy_provider::Provider as _;
use alloy_provider::ProviderBuilder;
use alloy_provider::RootProvider;
use alloy_pubsub::Subscription;
use alloy_rpc_types::Filter;
use alloy_rpc_types::Log;
use ethereum_types::H160;
use ethers::core::abi::Address;
use ethers::core::k256::ecdsa::SigningKey;
use ethers::core::types::transaction::eip2718::TypedTransaction;
use ethers::core::types::{Block, Eip1559TransactionRequest, TxHash};
use ethers::core::types::{Transaction, TransactionReceipt};
use ethers::middleware::SignerMiddleware;
use ethers::providers::{Http, Middleware, PendingTransaction, Provider};
use ethers::signers::Wallet;
use ethers::signers::{LocalWallet, Signer};
use futures::StreamExt;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use sov_cli::NodeClient;
use sov_modules_api::{Runtime, Spec};
use sov_test_utils::SimpleStorageContract;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const GAS: u64 = 9000000u64;
const MAX_FEE_PER_GAS: u64 = 100;
const MAX_PRIORITY_FEE_PER_GAS: u64 = 1;

pub struct TestClient {
    pub chain_id: u64,
    pub from_addr: Address,
    contract: SimpleStorageContract,
    pub_sub: RootProvider,
    pub client: SignerMiddleware<Provider<Http>, Wallet<SigningKey>>,
    node_client: NodeClient,
    rpc: WsClient,
    pub nonce: Arc<AtomicU64>,
}

async fn pubsub(conn_str: &str) -> RootProvider {
    ProviderBuilder::default().connect(conn_str).await.unwrap()
}

impl TestClient {
    pub async fn new(
        chain_id: u64,
        private_key: &str,
        contract: SimpleStorageContract,
        http_addr: std::net::SocketAddr,
    ) -> Self {
        let ws_conn_str = &format!("ws://127.0.0.1:{}/rpc", http_addr.port());
        let http_conn_str = &format!("http://127.0.0.1:{}/rpc", http_addr.port());

        let pub_sub = pubsub(ws_conn_str).await;

        let client = {
            let key = private_key
                .parse::<LocalWallet>()
                .unwrap()
                .with_chain_id(chain_id);

            let provider = Provider::try_from(http_conn_str).unwrap();

            SignerMiddleware::new_with_provider_chain(provider, key.clone())
                .await
                .unwrap()
        };

        let rpc = WsClientBuilder::default().build(ws_conn_str).await.unwrap();

        let node_client = NodeClient::new_at_localhost(http_addr.port())
            .await
            .unwrap();

        // Fetch initial nonce from the network
        let from_addr = client.address();
        let initial_nonce = client
            .get_transaction_count(from_addr, None)
            .await
            .unwrap()
            .as_u64();
        let nonce = Arc::new(AtomicU64::new(initial_nonce));

        Self {
            chain_id,
            from_addr,
            contract,
            pub_sub,
            client,
            node_client,
            rpc,
            nonce,
        }
    }

    fn default_request(&self) -> Eip1559TransactionRequest {
        Eip1559TransactionRequest::new()
            .from(self.from_addr)
            .chain_id(self.chain_id)
            .max_priority_fee_per_gas(MAX_PRIORITY_FEE_PER_GAS)
            .max_fee_per_gas(MAX_FEE_PER_GAS)
            .gas(GAS)
    }

    fn make_eip1559_tx(
        &self,
        to_address: Option<Address>,
        data: Option<ethers::core::types::Bytes>,
    ) -> TypedTransaction {
        // Get next nonce atomically
        let nonce = self.nonce.load(Ordering::SeqCst);
        let mut req = self.default_request().nonce(nonce);

        if let Some(data) = data {
            req = req.data(data)
        }

        if let Some(addr) = to_address {
            req = req.to(addr)
        }

        TypedTransaction::Eip1559(req)
    }

    pub async fn deploy_contract(
        &self,
    ) -> Result<PendingTransaction<'_, Http>, Box<dyn std::error::Error>> {
        let typed_transaction = self.make_eip1559_tx(None, Some(self.contract.byte_code()));
        let receipt_req = self.eth_send_transaction(typed_transaction).await?;

        Ok(receipt_req)
    }

    pub async fn deploy_contract_call(&self) -> Result<Bytes, Box<dyn std::error::Error>> {
        let typed_transaction = self.make_eip1559_tx(None, Some(self.contract.byte_code()));
        let receipt_req = self.eth_call(typed_transaction).await?;

        Ok(receipt_req)
    }

    pub async fn set_value_unsigned(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> PendingTransaction<'_, Http> {
        let typed_transaction = self.make_eip1559_tx(
            Some(contract_address),
            Some(self.contract.set_call_data(set_arg)),
        );

        self.eth_send_transaction(typed_transaction).await.unwrap()
    }

    pub async fn set_values(
        &self,
        contract_address: H160,
        set_args: Vec<u32>,
    ) -> Vec<PendingTransaction<'_, Http>> {
        let mut requests: Vec<_> = Vec::with_capacity(set_args.len());

        for set_arg in set_args.into_iter() {
            let typed_transaction = self.make_eip1559_tx(
                Some(contract_address),
                Some(self.contract.set_call_data(set_arg)),
            );

            requests.push(self.eth_send_transaction(typed_transaction).await.unwrap());
        }
        requests
    }

    pub async fn set_value(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> PendingTransaction<'_, Http> {
        let typed_transaction = self.make_eip1559_tx(
            Some(contract_address),
            Some(self.contract.set_call_data(set_arg)),
        );

        self.eth_send_transaction(typed_transaction).await.unwrap()
    }

    pub async fn set_value_call_and_estimate_gas(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let mut typed_transaction = self.make_eip1559_tx(
            Some(contract_address),
            Some(self.contract.set_call_data(set_arg)),
        );
        let gas = self.eth_estimate_gas(typed_transaction.clone()).await;

        typed_transaction.set_gas(gas);

        let response = self.eth_call(typed_transaction).await?;

        Ok(response)
    }

    pub async fn failing_call(
        &self,
        contract_address: H160,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let typed_transaction = self.make_eip1559_tx(
            Some(contract_address),
            Some(self.contract.failing_function_call_data()),
        );

        self.eth_call(typed_transaction).await
    }

    pub async fn always_reverts(
        &self,
        contract_address: H160,
    ) -> Result<PendingTransaction<'_, Http>, Box<dyn std::error::Error>> {
        let typed_transaction =
            self.make_eip1559_tx(Some(contract_address), Some(self.contract.always_revert()));

        self.eth_send_transaction(typed_transaction).await
    }

    pub async fn query_contract(
        &self,
        contract_address: H160,
    ) -> Result<ethereum_types::U256, Box<dyn std::error::Error>> {
        let typed_transaction =
            self.make_eip1559_tx(Some(contract_address), Some(self.contract.get_call_data()));

        let response = self.client.call(&typed_transaction, None).await?;

        let resp_array: [u8; 32] = response.to_vec().try_into().unwrap();
        Ok(ethereum_types::U256::from(resp_array))
    }

    #[allow(dead_code)]
    pub async fn eth_accounts(&self) -> Vec<Address> {
        self.client.get_accounts().await.unwrap()
    }

    pub async fn eth_send_transaction(
        &self,
        tx: TypedTransaction,
    ) -> Result<PendingTransaction<'_, Http>, Box<dyn std::error::Error>> {
        // Increment nonce
        let _ = self.nonce.fetch_add(1, Ordering::SeqCst);
        Ok(self.client.send_transaction(tx, None).await?)
    }

    pub async fn eth_chain_id(&self) -> u64 {
        self.client.get_chainid().await.unwrap().as_u64()
    }

    pub async fn eth_get_balance(&self, address: Address) -> ethereum_types::U256 {
        self.client.get_balance(address, None).await.unwrap()
    }

    pub async fn eth_get_storage_at(
        &self,
        address: Address,
        index: ethereum_types::U256,
    ) -> ethereum_types::U256 {
        self.rpc
            .request("eth_getStorageAt", rpc_params![address, index])
            .await
            .unwrap()
    }

    pub async fn eth_get_code(&self, address: Address) -> Vec<u8> {
        self.client.get_code(address, None).await.unwrap().to_vec()
    }

    pub async fn eth_get_transaction_count(&self, address: Address) -> u64 {
        let count = self
            .client
            .get_transaction_count(address, None)
            .await
            .unwrap();

        count.as_u64()
    }

    pub async fn eth_get_block_by_number(&self, block_number: Option<String>) -> Block<TxHash> {
        self.rpc
            .request("eth_getBlockByNumber", rpc_params![block_number, false])
            .await
            .unwrap()
    }

    pub async fn eth_call(
        &self,
        tx: TypedTransaction,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        self.rpc
            .request("eth_call", rpc_params![tx])
            .await
            .map_err(|e| e.into())
    }

    pub async fn eth_estimate_gas(&self, tx: TypedTransaction) -> u64 {
        let gas = self.client.estimate_gas(&tx, None).await.unwrap();
        gas.as_u64()
    }

    pub async fn send_transactions_and_wait_slot<S: Spec, Rt: Runtime<S>>(
        &self,
        transactions: &[sov_modules_api::transaction::Transaction<Rt, S>],
    ) -> anyhow::Result<()> {
        let mut slot_subscription = self.node_client.client.subscribe_slots().await?;

        self.node_client
            .client
            .send_txs_to_sequencer(transactions)
            .await?;

        let _ = slot_subscription.next().await;

        Ok(())
    }

    pub async fn send_eth(&self, reciever: H160, eth_value: u128) -> PendingTransaction<'_, Http> {
        let mut typed_transaction = self.make_eip1559_tx(Some(reciever), None);

        // Set the value for ETH transfer
        typed_transaction.set_value(eth_value);

        self.eth_send_transaction(typed_transaction).await.unwrap()
    }

    pub async fn receipt(&self, hash: TxHash) -> Option<TransactionReceipt> {
        self.client.get_transaction_receipt(hash).await.unwrap()
    }

    pub async fn transaction(&self, hash: TxHash) -> Option<Transaction> {
        self.client.get_transaction(hash).await.unwrap()
    }
}

impl TestClient {
    pub async fn alloy_deploy_contract(&self) -> alloy_primitives::Address {
        let typed_transaction = self.make_eip1559_tx(None, Some(self.contract.byte_code()));
        let addr = self
            .eth_send_transaction(typed_transaction)
            .await
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .contract_address
            .unwrap();

        alloy_primitives::Address::from_slice(addr.0.as_slice())
    }

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

    pub async fn alloy_set_value(
        &self,
        contract_address: alloy_primitives::Address,
        set_arg: u32,
    ) -> alloy_primitives::TxHash {
        let typed_transaction = self.make_eip1559_tx(
            Some(ethers::core::abi::Address::from_slice(
                contract_address.as_slice(),
            )),
            Some(self.contract.set_call_data(set_arg)),
        );

        let tx_hash = self
            .eth_send_transaction(typed_transaction)
            .await
            .unwrap()
            .tx_hash();

        alloy_primitives::TxHash::from_slice(&tx_hash.0)
    }

    pub async fn alloy_emit_logs(
        &self,
        contract_address: alloy_primitives::Address,
        topic: u32,
        nb_of_logs: u32,
    ) -> alloy_primitives::TxHash {
        let typed_transaction = self.make_eip1559_tx(
            Some(ethers::core::abi::Address::from_slice(
                contract_address.as_slice(),
            )),
            Some(self.contract.emit_logs(topic, nb_of_logs)),
        );

        let tx_hash = self
            .eth_send_transaction(typed_transaction)
            .await
            .unwrap()
            .tx_hash();

        alloy_primitives::TxHash::from_slice(&tx_hash.0)
    }

    pub async fn alloy_get_block_by_number(
        &self,
        block_number: Option<String>,
    ) -> alloy_rpc_types::Block<alloy_primitives::TxHash> {
        self.rpc
            .request("eth_getBlockByNumber", rpc_params![block_number, false])
            .await
            .unwrap()
    }

    pub async fn get_logs(&self, filter: &Filter) -> Vec<Log> {
        self.pub_sub.get_logs(filter).await.unwrap()
    }

    pub async fn block_number(&self) -> u64 {
        self.client.get_block_number().await.unwrap().as_u64()
    }

    pub async fn eth_gas_price(&self) -> u128 {
        self.client.get_gas_price().await.unwrap().as_u128()
    }
}
