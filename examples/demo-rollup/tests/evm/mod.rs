use std::net::SocketAddr;
use std::str::FromStr;

use ethereum_types::H160;
use ethers_core::abi::Address;
use ethers_core::k256::ecdsa::SigningKey;
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::Eip1559TransactionRequest;
use ethers_middleware::SignerMiddleware;
use ethers_providers::{Http, Middleware, PendingTransaction, Provider};
use ethers_signers::{LocalWallet, Signer, Wallet};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;
use sov_evm::smart_contracts::SimpleStorageContract;

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

    async fn set_value(
        &self,
        contract_address: H160,
        set_arg: u32,
        nonce: u64,
    ) -> PendingTransaction<'_, Http> {
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
        nonce: u64,
    ) -> Result<ethereum_types::U256, Box<dyn std::error::Error>> {
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

    async fn execute(self) -> Result<(), Box<dyn std::error::Error>> {
        let contract_address = {
            let deploy_contract_req = self.deploy_contract().await?;
            self.send_publish_batch_request().await;

            deploy_contract_req
                .await?
                .unwrap()
                .contract_address
                .unwrap()
        };

        let set_arg = 923;
        {
            let set_value_req = self.set_value(contract_address, set_arg, 1).await;
            self.send_publish_batch_request().await;
            set_value_req.await.unwrap();
        }

        {
            let get_arg = self.query_contract(contract_address, 2).await?;
            assert_eq!(set_arg, get_arg.as_u32());
        }

        // Create a blob with multiple transactions.
        let mut requests = Vec::default();
        let mut nonce = 2;
        for value in 100..103 {
            let set_value_req = self.set_value(contract_address, value, nonce).await;
            requests.push(set_value_req);
            nonce += 1
        }

        self.send_publish_batch_request().await;

        for req in requests {
            req.await.unwrap();
        }

        {
            let get_arg = self.query_contract(contract_address, nonce).await?;
            assert_eq!(102, get_arg.as_u32());
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
    test_client.execute().await
}

#[tokio::test]
async fn evm_tx_tests() -> Result<(), anyhow::Error> {
    let (port_tx, port_rx) = tokio::sync::oneshot::channel();

    let rollup_task = tokio::spawn(async {
        start_rollup(port_tx).await;
    });

    // Wait for rollup task to start:
    let port = port_rx.await.unwrap();
    send_tx_test_to_eth(port).await.unwrap();
    rollup_task.abort();
    Ok(())
}
