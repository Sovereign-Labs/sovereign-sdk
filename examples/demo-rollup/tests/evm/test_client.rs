use std::net::SocketAddr;

use ethereum_types::H160;
use ethers_core::abi::Address;
use ethers_core::k256::ecdsa::SigningKey;
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::{
    Block, Eip1559TransactionRequest, Transaction, TransactionRequest, TxHash,
};
use ethers_middleware::SignerMiddleware;
use ethers_providers::{Http, Middleware, PendingTransaction, Provider};
use ethers_signers::{Signer, Wallet};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;
use reth_primitives::Bytes;
use sov_evm::SimpleStorageContract;

const MAX_FEE_PER_GAS: u64 = 100000001;

pub(crate) struct TestClient {
    chain_id: u64,
    from_addr: Address,
    contract: SimpleStorageContract,
    client: SignerMiddleware<Provider<Http>, Wallet<SigningKey>>,
    http_client: HttpClient,
}

impl TestClient {
    #[allow(dead_code)]
    pub(crate) async fn new(
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

    pub(crate) async fn send_publish_batch_request(&self) {
        let _: String = self
            .http_client
            .request("eth_publishBatch", rpc_params![])
            .await
            .unwrap();
    }

    pub(crate) async fn deploy_contract(
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

    pub(crate) async fn deploy_contract_call(&self) -> Result<Bytes, Box<dyn std::error::Error>> {
        let req = Eip1559TransactionRequest::new()
            .from(self.from_addr)
            .chain_id(self.chain_id)
            .nonce(0u64)
            .max_priority_fee_per_gas(10u64)
            .max_fee_per_gas(MAX_FEE_PER_GAS)
            .gas(900000u64)
            .data(self.contract.byte_code());

        let typed_transaction = TypedTransaction::Eip1559(req);

        let receipt_req = self.eth_call(typed_transaction, None).await?;

        Ok(receipt_req)
    }

    pub(crate) async fn set_value_unsigned(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> PendingTransaction<'_, Http> {
        // Tx without gas_limit should estimate and include it in send_transaction endpoint
        // Tx without nonce should fetch and include it in send_transaction endpoint
        let req = Eip1559TransactionRequest::new()
            .from(self.from_addr)
            .to(contract_address)
            .chain_id(self.chain_id)
            .data(self.contract.set_call_data(set_arg))
            .max_priority_fee_per_gas(10u64)
            .max_fee_per_gas(MAX_FEE_PER_GAS);

        let typed_transaction = TypedTransaction::Eip1559(req);

        self.eth_send_transaction(typed_transaction).await
    }

    pub(crate) async fn set_value(
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

    pub(crate) async fn set_value_call(
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

        // Estimate gas on rpc
        let gas = self
            .eth_estimate_gas(typed_transaction, Some("latest".to_owned()))
            .await;

        // Call with the estimated gas
        let req = req.gas(gas);
        let typed_transaction = TypedTransaction::Legacy(req);

        let response = self
            .eth_call(typed_transaction, Some("latest".to_owned()))
            .await?;

        Ok(response)
    }

    pub(crate) async fn failing_call(
        &self,
        contract_address: H160,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let nonce = self.eth_get_transaction_count(self.from_addr).await;

        // Any type of transaction can be used for eth_call
        let req = Eip1559TransactionRequest::new()
            .from(self.from_addr)
            .to(contract_address)
            .chain_id(self.chain_id)
            .nonce(nonce)
            .data(self.contract.failing_function_call_data())
            .max_priority_fee_per_gas(10u64)
            .max_fee_per_gas(MAX_FEE_PER_GAS)
            .gas(900000u64);

        let typed_transaction = TypedTransaction::Eip1559(req);

        self.eth_call(typed_transaction, Some("latest".to_owned()))
            .await
    }

    pub(crate) async fn query_contract(
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

    pub(crate) async fn eth_accounts(&self) -> Vec<Address> {
        self.http_client
            .request("eth_accounts", rpc_params![])
            .await
            .unwrap()
    }

    pub(crate) async fn eth_send_transaction(
        &self,
        tx: TypedTransaction,
    ) -> PendingTransaction<'_, Http> {
        self.client
            .provider()
            .send_transaction(tx, None)
            .await
            .unwrap()
    }

    pub(crate) async fn eth_chain_id(&self) -> u64 {
        let chain_id: ethereum_types::U64 = self
            .http_client
            .request("eth_chainId", rpc_params![])
            .await
            .unwrap();

        chain_id.as_u64()
    }

    pub(crate) async fn eth_get_balance(&self, address: Address) -> ethereum_types::U256 {
        self.http_client
            .request("eth_getBalance", rpc_params![address, "latest"])
            .await
            .unwrap()
    }

    pub(crate) async fn eth_get_storage_at(
        &self,
        address: Address,
        index: ethereum_types::U256,
    ) -> ethereum_types::U256 {
        self.http_client
            .request("eth_getStorageAt", rpc_params![address, index, "latest"])
            .await
            .unwrap()
    }

    pub(crate) async fn eth_get_code(&self, address: Address) -> Bytes {
        self.http_client
            .request("eth_getCode", rpc_params![address, "latest"])
            .await
            .unwrap()
    }

    pub(crate) async fn eth_get_transaction_count(&self, address: Address) -> u64 {
        let count: ethereum_types::U64 = self
            .http_client
            .request("eth_getTransactionCount", rpc_params![address, "latest"])
            .await
            .unwrap();

        count.as_u64()
    }

    pub(crate) async fn eth_get_block_by_number(
        &self,
        block_number: Option<String>,
    ) -> Block<TxHash> {
        self.http_client
            .request("eth_getBlockByNumber", rpc_params![block_number, false])
            .await
            .unwrap()
    }

    pub(crate) async fn eth_get_block_by_number_with_detail(
        &self,
        block_number: Option<String>,
    ) -> Block<Transaction> {
        self.http_client
            .request("eth_getBlockByNumber", rpc_params![block_number, true])
            .await
            .unwrap()
    }

    pub(crate) async fn eth_call(
        &self,
        tx: TypedTransaction,
        block_number: Option<String>,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        self.http_client
            .request("eth_call", rpc_params![tx, block_number])
            .await
            .map_err(|e| e.into())
    }

    pub(crate) async fn eth_estimate_gas(
        &self,
        tx: TypedTransaction,
        block_number: Option<String>,
    ) -> u64 {
        let gas: ethereum_types::U64 = self
            .http_client
            .request("eth_estimateGas", rpc_params![tx, block_number])
            .await
            .unwrap();

        gas.as_u64()
    }

    pub(crate) async fn execute(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Nonce should be 0 in genesis
        let nonce = self.eth_get_transaction_count(self.from_addr).await;
        assert_eq!(0, nonce);

        // Balance should be > 0 in genesis
        let balance = self.eth_get_balance(self.from_addr).await;
        assert!(balance > ethereum_types::U256::zero());

        let (contract_address, runtime_code) = {
            let runtime_code = self.deploy_contract_call().await?;

            let deploy_contract_req = self.deploy_contract().await?;
            self.send_publish_batch_request().await;

            let contract_address = deploy_contract_req
                .await?
                .unwrap()
                .contract_address
                .unwrap();

            (contract_address, runtime_code)
        };

        // Assert contract deployed correctly
        let code = self.eth_get_code(contract_address).await;
        // code has natural following 0x00 bytes, so we need to trim it
        assert_eq!(code.to_vec()[..runtime_code.len()], runtime_code.to_vec());

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

        let get_arg = self.query_contract(contract_address).await?;
        assert_eq!(set_arg, get_arg.as_u32());

        // Assert storage slot is set
        let storage_slot = 0x0;
        let storage_value = self
            .eth_get_storage_at(contract_address, storage_slot.into())
            .await;
        assert_eq!(storage_value, ethereum_types::U256::from(set_arg));

        // Check that the second block has published
        // None should return the latest block
        // It should have a single transaction, setting the value
        let latest_block = self.eth_get_block_by_number_with_detail(None).await;
        assert_eq!(latest_block.number.unwrap().as_u64(), 2);
        assert_eq!(latest_block.transactions.len(), 1);
        assert_eq!(latest_block.transactions[0].hash, tx_hash);

        // This should just pass without error
        self.set_value_call(contract_address, set_arg)
            .await
            .unwrap();

        // This call should fail because function does not exist
        let failing_call = self.failing_call(contract_address).await;
        assert!(failing_call.is_err());

        // Create a blob with multiple transactions.
        let mut requests = Vec::default();
        for value in 100..103 {
            let set_value_req = self.set_value(contract_address, value).await;
            requests.push(set_value_req);
        }

        self.send_publish_batch_request().await;

        // second block
        self.send_publish_batch_request().await;

        let first_block = self.eth_get_block_by_number(Some("0".to_owned())).await;
        let second_block = self.eth_get_block_by_number(Some("1".to_owned())).await;

        println!("first_block: {:?}", first_block);
        println!("second_block: {:?}", second_block);

        // assert parent hash
        assert_eq!(
            first_block.hash.unwrap(),
            second_block.parent_hash,
            "Parent hash should be the hash of the previous block"
        );

        for req in requests {
            req.await.unwrap();
        }

        {
            let get_arg = self.query_contract(contract_address).await?;
            assert_eq!(102, get_arg.as_u32());
        }

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
