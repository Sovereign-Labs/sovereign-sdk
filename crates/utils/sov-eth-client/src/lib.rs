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
use ethers::core::types::{Block, Eip1559TransactionRequest, TransactionRequest, TxHash};
use ethers::core::types::{Transaction, TransactionReceipt};
use ethers::middleware::signer::SignerMiddlewareError;
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

const GAS: u64 = 900000u64;
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

        Self {
            chain_id,
            from_addr: client.address(),
            contract,
            pub_sub,
            client,
            node_client,
            rpc,
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
        nonce: u64,
        to_address: Option<Address>,
        data: Option<ethers::core::types::Bytes>,
    ) -> TypedTransaction {
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
        let typed_transaction = self.make_eip1559_tx(0, None, Some(self.contract.byte_code()));
        let receipt_req = self
            .client
            .send_transaction(typed_transaction, None)
            .await?;

        Ok(receipt_req)
    }

    pub async fn deploy_contract_call(&self) -> Result<Bytes, Box<dyn std::error::Error>> {
        let typed_transaction = self.make_eip1559_tx(0, None, Some(self.contract.byte_code()));
        let receipt_req = self.eth_call(typed_transaction).await?;

        Ok(receipt_req)
    }

    pub async fn set_value_unsigned(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> PendingTransaction<'_, Http> {
        // TODO: Re-evaluate if it's still needed after we migrate from ethers
        let nonce = self.eth_get_transaction_count(self.from_addr).await;
        tracing::info!(from = %self.from_addr, nonce, "SmartContract::set_value");

        let typed_transaction = self.make_eip1559_tx(
            nonce,
            Some(contract_address),
            Some(self.contract.set_call_data(set_arg)),
        );

        self.eth_send_transaction(typed_transaction).await
    }

    pub async fn set_values(
        &self,
        contract_address: H160,
        set_args: Vec<u32>,
    ) -> Vec<PendingTransaction<'_, Http>> {
        let mut requests: Vec<_> = Vec::with_capacity(set_args.len());
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        for (i, set_arg) in set_args.into_iter().enumerate() {
            let typed_transaction = self.make_eip1559_tx(
                nonce + (i as u64),
                Some(contract_address),
                Some(self.contract.set_call_data(set_arg)),
            );

            requests.push(
                self.client
                    .send_transaction(typed_transaction, None)
                    .await
                    .unwrap(),
            );
        }
        requests
    }

    pub async fn set_value(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> PendingTransaction<'_, Http> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;
        tracing::info!(from = %self.from_addr, nonce, "SmartContract::set_value");

        let typed_transaction = self.make_eip1559_tx(
            nonce,
            Some(contract_address),
            Some(self.contract.set_call_data(set_arg)),
        );

        self.client
            .send_transaction(typed_transaction, None)
            .await
            .unwrap()
    }

    pub async fn emit_one_log(&self, contract_address: H160) -> PendingTransaction<'_, Http> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;
        tracing::info!(from = %self.from_addr, nonce, "SmartContract::set_value");

        let typed_transaction = self.make_eip1559_tx(
            nonce,
            Some(contract_address),
            Some(self.contract.emit_one_log()),
        );

        self.client
            .send_transaction(typed_transaction, None)
            .await
            .unwrap()
    }

    pub async fn set_value_call_and_estimate_gas(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        // Any type of transaction can be used for eth_call
        let req = TransactionRequest::new()
            .from(self.from_addr)
            .to(contract_address)
            .chain_id(self.chain_id)
            .nonce(nonce)
            .data(self.contract.set_call_data(set_arg))
            .gas_price(10u64);

        let typed_transaction = TypedTransaction::Legacy(req.clone());

        // Estimate gas on RPC
        let gas = self.eth_estimate_gas(typed_transaction).await;

        // Call with the estimated gas
        let req = req.gas(gas);
        let typed_transaction = TypedTransaction::Legacy(req);

        let response = self.eth_call(typed_transaction).await?;

        Ok(response)
    }

    pub async fn failing_call(
        &self,
        contract_address: H160,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        let typed_transaction = self.make_eip1559_tx(
            nonce,
            Some(contract_address),
            Some(self.contract.failing_function_call_data()),
        );

        self.eth_call(typed_transaction).await
    }

    pub async fn always_reverts(
        &self,
        contract_address: H160,
    ) -> Result<
        PendingTransaction<'_, Http>,
        SignerMiddlewareError<Provider<Http>, Wallet<SigningKey>>,
    > {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        let typed_transaction = self.make_eip1559_tx(
            nonce,
            Some(contract_address),
            Some(self.contract.always_revert()),
        );

        self.client.send_transaction(typed_transaction, None).await
    }

    pub async fn query_contract(
        &self,
        contract_address: H160,
    ) -> Result<ethereum_types::U256, Box<dyn std::error::Error>> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        let typed_transaction = self.make_eip1559_tx(
            nonce,
            Some(contract_address),
            Some(self.contract.get_call_data()),
        );

        let response = self.client.call(&typed_transaction, None).await?;

        let resp_array: [u8; 32] = response.to_vec().try_into().unwrap();
        Ok(ethereum_types::U256::from(resp_array))
    }

    #[allow(dead_code)]
    pub async fn eth_accounts(&self) -> Vec<Address> {
        self.client.get_accounts().await.unwrap()
    }

    pub async fn eth_send_transaction(&self, tx: TypedTransaction) -> PendingTransaction<'_, Http> {
        self.client
            .provider()
            .send_transaction(tx, None)
            .await
            .unwrap()
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

    pub async fn eth_gas_price(&self) -> u128 {
        self.client.get_gas_price().await.unwrap().as_u128()
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
        let nonce = self.eth_get_transaction_count(self.from_addr).await;
        tracing::info!(from = %self.from_addr, nonce, "SmartContract::set_value");

        let req = self
            .default_request()
            .nonce(nonce)
            .to(reciever)
            .value(eth_value);

        let typed_transaction = TypedTransaction::Eip1559(req);

        self.client
            .send_transaction(typed_transaction, None)
            .await
            .unwrap()
    }

    pub async fn receipt(&self, hash: TxHash) -> Option<TransactionReceipt> {
        self.client.get_transaction_receipt(hash).await.unwrap()
    }

    pub async fn transaction(&self, hash: TxHash) -> Option<Transaction> {
        self.client.get_transaction(hash).await.unwrap()
    }

    pub async fn block_number(&self) -> u64 {
        self.client.get_block_number().await.unwrap().as_u64()
    }

    pub async fn subscribe_logs(&self) -> Subscription<Log> {
        let filter = Filter::new();
        self.pub_sub.subscribe_logs(&filter).await.unwrap()
    }
}
