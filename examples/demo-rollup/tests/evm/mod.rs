use std::net::SocketAddr;
use std::str::FromStr;

use ethereum_types::H160;
use ethers_core::abi::Address;
use ethers_core::k256::ecdsa::SigningKey;
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::{Block, Eip1559TransactionRequest, Transaction, TxHash};
use ethers_middleware::SignerMiddleware;
use ethers_providers::{Http, Middleware, PendingTransaction, Provider};
use ethers_signers::{LocalWallet, Signer, Wallet};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;
use sov_evm::smart_contracts::SimpleStorageContract;
use sov_risc0_adapter::host::Risc0Host;

use super::test_helpers::start_rollup;

const MAX_FEE_PER_GAS: u64 = 100000001;

struct TestClient {
    chain_id: u64,
    from_addr: Address,
    contract: SimpleStorageContract,
    client: SignerMiddleware<Provider<Http>, Wallet<SigningKey>>,
    http_client: HttpClient,
}

impl TestClient {
    #[allow(dead_code)]
    async fn new(
        chain_id: u64,
        key: Wallet<SigningKey>,
        from_addr: Address,
        contract: SimpleStorageContract,
        rpc_addr: std::net::SocketAddr,
    ) -> Self {
        let host = format!("http://localhost:{}", rpc_addr.port());

        let provider = Provider::try_from(&host).unwrap();
        let client = SignerMiddleware::new_with_provider_chain(provider, key)
            .await
            .unwrap();

        let http_client = HttpClientBuilder::default().build(host).unwrap();

        Self {
            chain_id,
            from_addr,
            contract,
            client,
            http_client,
        }
    }

    async fn send_publish_batch_request(&self) {
        let _: String = self
            .http_client
            .request("eth_publishBatch", rpc_params![])
            .await
            .unwrap();
    }

    async fn deploy_contract(
        &self,
    ) -> Result<PendingTransaction<'_, Http>, Box<dyn std::error::Error>> {
        let req = Eip1559TransactionRequest::new()
            .from(self.from_addr)
            .chain_id(self.chain_id)
            .nonce(0u64)
            .max_priority_fee_per_gas(10u64)
            .max_fee_per_gas(MAX_FEE_PER_GAS)
            .gas(900000u64)
            .data(self.contract.byte_code());

        let typed_transaction = TypedTransaction::Eip1559(req);

        let receipt_req = self
            .client
            .send_transaction(typed_transaction, None)
            .await?;

        Ok(receipt_req)
    }

    async fn set_value_unsigned(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> PendingTransaction<'_, Http> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        let req = Eip1559TransactionRequest::new()
            .from(self.from_addr)
            .to(contract_address)
            .chain_id(self.chain_id)
            .nonce(nonce)
            .data(self.contract.set_call_data(set_arg))
            .max_priority_fee_per_gas(10u64)
            .max_fee_per_gas(MAX_FEE_PER_GAS)
            .gas(900000u64);

        let typed_transaction = TypedTransaction::Eip1559(req);

        self.eth_send_transaction(typed_transaction).await
    }

    async fn set_value(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> PendingTransaction<'_, Http> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        let req = Eip1559TransactionRequest::new()
            .from(self.from_addr)
            .to(contract_address)
            .chain_id(self.chain_id)
            .nonce(nonce)
            .data(self.contract.set_call_data(set_arg))
            .max_priority_fee_per_gas(10u64)
            .max_fee_per_gas(MAX_FEE_PER_GAS)
            .gas(900000u64);

        let typed_transaction = TypedTransaction::Eip1559(req);

        self.client
            .send_transaction(typed_transaction, None)
            .await
            .unwrap()
    }

    async fn query_contract(
        &self,
        contract_address: H160,
    ) -> Result<ethereum_types::U256, Box<dyn std::error::Error>> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        let req = Eip1559TransactionRequest::new()
            .from(self.from_addr)
            .to(contract_address)
            .chain_id(self.chain_id)
            .nonce(nonce)
            .data(self.contract.get_call_data())
            .gas(900000u64);

        let typed_transaction = TypedTransaction::Eip1559(req);

        let response = self.client.call(&typed_transaction, None).await?;

        let resp_array: [u8; 32] = response.to_vec().try_into().unwrap();
        Ok(ethereum_types::U256::from(resp_array))
    }

    #[cfg(feature = "local")]
    async fn eth_accounts(&self) -> Vec<Address> {
        self.http_client
            .request("eth_accounts", rpc_params![])
            .await
            .unwrap()
    }

    #[cfg(feature = "local")]
    async fn eth_send_transaction(&self, tx: TypedTransaction) -> PendingTransaction<'_, Http> {
        self.client
            .provider()
            .send_transaction(tx, None)
            .await
            .unwrap()
    }

    async fn eth_chain_id(&self) -> u64 {
        let chain_id: ethereum_types::U64 = self
            .http_client
            .request("eth_chainId", rpc_params![])
            .await
            .unwrap();

        chain_id.as_u64()
    }

    async fn eth_get_transaction_count(&self, address: Address) -> u64 {
        let count: ethereum_types::U64 = self
            .http_client
            .request("eth_getTransactionCount", rpc_params![address, "latest"])
            .await
            .unwrap();

        count.as_u64()
    }

    async fn eth_get_block_by_number(&self, block_number: Option<String>) -> Block<TxHash> {
        self.http_client
            .request("eth_getBlockByNumber", rpc_params![block_number, false])
            .await
            .unwrap()
    }

    async fn eth_get_block_by_number_with_detail(
        &self,
        block_number: Option<String>,
    ) -> Block<Transaction> {
        self.http_client
            .request("eth_getBlockByNumber", rpc_params![block_number, true])
            .await
            .unwrap()
    }

    async fn execute(self) -> Result<(), Box<dyn std::error::Error>> {
        // Nonce should be 0 in genesis
        let nonce = self.eth_get_transaction_count(self.from_addr).await;
        assert_eq!(0, nonce);

        let contract_address = {
            let deploy_contract_req = self.deploy_contract().await?;
            self.send_publish_batch_request().await;

            deploy_contract_req
                .await?
                .unwrap()
                .contract_address
                .unwrap()
        };

        // Nonce should be 1 after the deploy
        let nonce = self.eth_get_transaction_count(self.from_addr).await;
        assert_eq!(1, nonce);

        // Check that the first block has published
        // It should have a single transaction, deploying the contract
        let first_block = self.eth_get_block_by_number(Some("1".to_owned())).await;
        assert_eq!(first_block.number.unwrap().as_u64(), 1);
        assert_eq!(first_block.transactions.len(), 1);

        let set_arg = 923;
        let tx_hash = {
            let set_value_req = self.set_value(contract_address, set_arg).await;
            self.send_publish_batch_request().await;
            set_value_req.await.unwrap().unwrap().transaction_hash
        };

        {
            let get_arg = self.query_contract(contract_address).await?;
            assert_eq!(set_arg, get_arg.as_u32());
        }

        // Check that the second block has published
        // None should return the latest block
        // It should have a single transaction, setting the value
        let latest_block = self.eth_get_block_by_number_with_detail(None).await;
        assert_eq!(latest_block.number.unwrap().as_u64(), 2);
        assert_eq!(latest_block.transactions.len(), 1);
        assert_eq!(latest_block.transactions[0].hash, tx_hash);

        // Create a blob with multiple transactions.
        let mut requests = Vec::default();
        for value in 100..103 {
            let set_value_req = self.set_value(contract_address, value).await;
            requests.push(set_value_req);
        }

        self.send_publish_batch_request().await;

        for req in requests {
            req.await.unwrap();
        }

        {
            let get_arg = self.query_contract(contract_address).await?;
            assert_eq!(102, get_arg.as_u32());
        }

        #[cfg(feature = "local")]
        {
            let value = 103;

            let tx_hash = {
                let set_value_req = self.set_value_unsigned(contract_address, value).await;
                self.send_publish_batch_request().await;
                set_value_req.await.unwrap().unwrap().transaction_hash
            };

            let latest_block = self.eth_get_block_by_number(None).await;
            assert_eq!(latest_block.transactions.len(), 1);
            assert_eq!(latest_block.transactions[0], tx_hash);

            let get_arg = self.query_contract(contract_address).await?;
            assert_eq!(value, get_arg.as_u32());
        }

        Ok(())
    }
}

async fn send_tx_test_to_eth(rpc_address: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    let chain_id: u64 = 1;
    let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
        .parse::<LocalWallet>()
        .unwrap()
        .with_chain_id(chain_id);

    let contract = SimpleStorageContract::default();

    let from_addr = Address::from_str("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap();

    let test_client = TestClient::new(chain_id, key, from_addr, contract, rpc_address).await;

    #[cfg(feature = "local")]
    {
        let etc_accounts = test_client.eth_accounts().await;
        assert_eq!(vec![from_addr], etc_accounts);
    }

    let eth_chain_id = test_client.eth_chain_id().await;
    assert_eq!(chain_id, eth_chain_id);

    // No block exists yet
    let latest_block = test_client
        .eth_get_block_by_number(Some("latest".to_owned()))
        .await;
    let earliest_block = test_client
        .eth_get_block_by_number(Some("earliest".to_owned()))
        .await;

    assert_eq!(latest_block, earliest_block);
    assert_eq!(latest_block.number.unwrap().as_u64(), 0);

    test_client.execute().await
}

#[cfg(feature = "experimental")]
#[tokio::test]
async fn evm_tx_tests() -> Result<(), anyhow::Error> {
    let (port_tx, port_rx) = tokio::sync::oneshot::channel();

    let rollup_task = tokio::spawn(async {
        // Don't provide a prover since the EVM is not currently provable
        start_rollup::<Risc0Host<'static>>(port_tx, None).await;
    });

    // Wait for rollup task to start:
    let port = port_rx.await.unwrap();
    send_tx_test_to_eth(port).await.unwrap();
    rollup_task.abort();
    Ok(())
}
