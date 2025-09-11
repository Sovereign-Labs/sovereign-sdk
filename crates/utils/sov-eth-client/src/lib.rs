#![allow(missing_docs)]

use ethers::core::types::Block;
use ethers::types::Res;
use futures::StreamExt;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use sov_cli::NodeClient;
use sov_modules_api::{Runtime, Spec};
use sov_test_utils::SimpleStorageContract;

use alloy::network::Ethereum;
use alloy::network::EthereumWallet;
use alloy::primitives::U256;
use alloy::providers::Provider as AlloyProvider;
use alloy::providers::ProviderBuilder;
use alloy::providers::{
    fillers::{ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, WalletFiller},
    Identity, RootProvider,
};
use alloy::providers::{PendingTransactionBuilder, PendingTransactionError};
use alloy::rpc::types::eth::transaction::TransactionRequest;
use alloy::rpc::types::eth::Transaction;
use alloy::rpc::types::eth::TransactionReceipt;
use alloy::rpc::types::TransactionInput;
use alloy::signers::local::PrivateKeySigner;
use alloy::transports::TransportResult;
use alloy_primitives::TxHash;
use alloy_primitives::{Address, Bytes};

type PubsubSigner = FillProvider<
    JoinFill<
        JoinFill<
            Identity,
            JoinFill<
                GasFiller,
                JoinFill<
                    alloy::providers::fillers::BlobGasFiller,
                    JoinFill<NonceFiller, ChainIdFiller>,
                >,
            >,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider,
    Ethereum,
>;

const GAS: u64 = 900000u64;
const MAX_FEE_PER_GAS: u64 = 100;
const MAX_PRIORITY_FEE_PER_GAS: u64 = 1;

pub struct TestClient {
    pub chain_id: u64,
    pub from_addr: Address,
    contract: SimpleStorageContract,
    pub client: PubsubSigner, //SignerMiddleware<Provider<Http>, Wallet<SigningKey>>,
    node_client: NodeClient,
    rpc: WsClient,
}

impl TestClient {
    pub async fn new(
        chain_id: u64,
        private_key: &str,
        contract: SimpleStorageContract,
        http_addr: std::net::SocketAddr,
    ) -> Self {
        let signer: PrivateKeySigner = private_key.parse().unwrap();
        let from_addr = signer.address();
        let eth_wallet = EthereumWallet::new(signer);

        let conn_str = &format!("ws://127.0.0.1:{}/rpc", http_addr.port());

        let provider: PubsubSigner = ProviderBuilder::new()
            .wallet(eth_wallet)
            .connect(conn_str)
            .await
            .unwrap();

        // let provider =
        //     Provider::try_from(&format!("http://127.0.0.1:{}/rpc", http_addr.port())).unwrap();
        let client = provider;

        // SignerMiddleware::new_with_provider_chain(provider, key.clone())
        //    .await
        //    .unwrap();

        let rpc = WsClientBuilder::default()
            .build(&format!("ws://127.0.0.1:{}/rpc", http_addr.port()))
            .await
            .unwrap();

        let node_client = NodeClient::new_at_localhost(http_addr.port())
            .await
            .unwrap();

        Self {
            chain_id,
            from_addr,
            contract,
            client,
            node_client,
            rpc,
        }
    }

    fn default_request(&self) -> TransactionRequest {
        let mut req = TransactionRequest::default()
            .from(self.from_addr)
            .max_priority_fee_per_gas(MAX_PRIORITY_FEE_PER_GAS as u128)
            .max_fee_per_gas(MAX_FEE_PER_GAS as u128)
            .gas_limit(GAS);

        req.chain_id = Some(self.chain_id);
        req
    }

    fn make_eip1559_tx(
        &self,
        nonce: u64,
        to_address: Option<Address>,
        data: Option<ethers::core::types::Bytes>,
    ) -> TransactionRequest {
        let data = data.map(|d| Bytes(d.0));
        let mut req = self.default_request().nonce(nonce);

        if let Some(data) = data {
            req = req.input(TransactionInput::new(data))
        }

        if let Some(addr) = to_address {
            req = req.to(addr)
        }

        req
    }

    pub async fn deploy_contract(&self) -> TransactionReceipt {
        let typed_transaction = self.make_eip1559_tx(0, None, Some(self.contract.byte_code()));
        let rec = self
            .client
            .send_transaction(typed_transaction)
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();

        rec
    }

    pub async fn deploy_contract_call(&self) -> Result<Bytes, Box<dyn std::error::Error>> {
        let typed_transaction = self.make_eip1559_tx(0, None, Some(self.contract.byte_code()));
        let receipt_req = self.eth_call(typed_transaction).await?;

        Ok(receipt_req)
    }

    pub async fn set_value_unsigned(
        &self,
        contract_address: Address,
        set_arg: u32,
    ) -> PendingTransactionBuilder<Ethereum> {
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
        contract_address: Address,
        set_args: Vec<u32>,
    ) -> Vec<PendingTransactionBuilder<Ethereum>> {
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
                    .send_transaction(typed_transaction)
                    .await
                    .unwrap(),
            );
        }
        requests
    }

    pub async fn set_value(
        &self,
        contract_address: Address,
        set_arg: u32,
    ) -> Result<TransactionReceipt, PendingTransactionError> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;
        tracing::info!(from = %self.from_addr, nonce, "SmartContract::set_value");

        let typed_transaction = self.make_eip1559_tx(
            nonce,
            Some(contract_address),
            Some(self.contract.set_call_data(set_arg)),
        );

        self.client
            .send_transaction(typed_transaction)
            .await
            .unwrap()
            .watch()
    }

    pub async fn emit_one_log(
        &self,
        contract_address: Address,
    ) -> PendingTransactionBuilder<Ethereum> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;
        tracing::info!(from = %self.from_addr, nonce, "SmartContract::set_value");

        let typed_transaction = self.make_eip1559_tx(
            nonce,
            Some(contract_address),
            Some(self.contract.emit_one_log()),
        );

        self.client
            .send_transaction(typed_transaction)
            .await
            .unwrap()
    }

    pub async fn set_value_call_and_estimate_gas(
        &self,
        contract_address: Address,
        set_arg: u32,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        // Any type of transaction can be used for eth_call
        let mut req = TransactionRequest::default()
            .from(self.from_addr)
            .to(contract_address)
            .nonce(nonce)
            .gas_price(10u128)
            .input(TransactionInput::new(Bytes(
                self.contract.set_call_data(set_arg).0,
            )));

        req.chain_id = Some(self.chain_id);

        let mut new_req = req.clone();
        // Estimate gas on RPC
        let gas = self.eth_estimate_gas(req).await;

        // Call with the estimated gas
        new_req.gas = Some(gas);

        let response = self.eth_call(new_req).await?;

        Ok(response)
    }

    pub async fn failing_call(
        &self,
        contract_address: Address,
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
        contract_address: Address,
    ) -> PendingTransactionBuilder<Ethereum> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        let typed_transaction = self.make_eip1559_tx(
            nonce,
            Some(contract_address),
            Some(self.contract.always_revert()),
        );

        self.client
            .send_transaction(typed_transaction)
            .await
            .unwrap()
    }

    pub async fn query_contract(
        &self,
        contract_address: Address,
    ) -> Result<ethereum_types::U256, Box<dyn std::error::Error>> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        let typed_transaction = self.make_eip1559_tx(
            nonce,
            Some(contract_address),
            Some(self.contract.get_call_data()),
        );

        let response = self.client.call(typed_transaction).await?;

        let resp_array: [u8; 32] = response.to_vec().try_into().unwrap();
        Ok(ethereum_types::U256::from(resp_array))
    }

    #[allow(dead_code)]
    pub async fn eth_accounts(&self) -> Vec<Address> {
        self.client.get_accounts().await.unwrap()
    }

    pub async fn eth_send_transaction(
        &self,
        tx: TransactionRequest,
    ) -> PendingTransactionBuilder<Ethereum> {
        self.client.send_transaction(tx).await.unwrap()
    }

    pub async fn eth_chain_id(&self) -> u64 {
        self.client.get_chain_id().await.unwrap()
    }

    pub async fn eth_get_balance(&self, address: Address) -> alloy_primitives::Uint<256, 4> {
        self.client.get_balance(address).await.unwrap()
    }

    pub async fn eth_get_storage_at(
        &self,
        address: Address,
        index: alloy_primitives::Uint<256, 4>,
    ) -> alloy_primitives::Uint<256, 4> {
        self.rpc
            .request("eth_getStorageAt", rpc_params![address, index])
            .await
            .unwrap()
    }

    pub async fn eth_get_code(&self, address: Address) -> Vec<u8> {
        self.client.get_code_at(address).await.unwrap().to_vec()
    }

    pub async fn eth_get_transaction_count(&self, address: Address) -> u64 {
        let count = self.client.get_transaction_count(address).await.unwrap();
        count
    }

    pub async fn eth_gas_price(&self) -> u128 {
        self.client.get_gas_price().await.unwrap()
    }

    pub async fn eth_get_block_by_number(&self, block_number: Option<String>) -> Block<TxHash> {
        self.rpc
            .request("eth_getBlockByNumber", rpc_params![block_number, false])
            .await
            .unwrap()
    }

    pub async fn eth_call(
        &self,
        tx: TransactionRequest,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        self.rpc
            .request("eth_call", rpc_params![tx])
            .await
            .map_err(|e| e.into())
    }

    pub async fn eth_estimate_gas(&self, tx: TransactionRequest) -> u64 {
        let gas = self.client.estimate_gas(tx).await.unwrap();
        gas
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

    pub async fn send_eth(
        &self,
        reciever: Address,
        eth_value: u128,
    ) -> PendingTransactionBuilder<Ethereum> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;
        tracing::info!(from = %self.from_addr, nonce, "SmartContract::set_value");

        let value = U256::from(eth_value);
        let req = self
            .default_request()
            .nonce(nonce)
            .to(reciever)
            .value(value);

        self.client.send_transaction(req).await.unwrap()
    }

    pub async fn receipt(&self, hash: TxHash) -> Option<TransactionReceipt> {
        self.client.get_transaction_receipt(hash).await.unwrap()
    }

    pub async fn transaction(&self, hash: TxHash) -> Option<Transaction> {
        self.client.get_transaction_by_hash(hash).await.unwrap()
    }

    pub async fn block_number(&self) -> u64 {
        self.client.get_block_number().await.unwrap()
    }
}
