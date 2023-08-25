use std::net::SocketAddr;
use std::str::FromStr;

use ethers_core::abi::Address;
use ethers_core::k256::ecdsa::SigningKey;
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::Eip1559TransactionRequest;
use ethers_middleware::SignerMiddleware;
use ethers_providers::{Http, Middleware, Provider};
use ethers_signers::{LocalWallet, Signer, Wallet};
use sov_evm::smart_contracts::SimpleStorageContract;

use crate::test_helpers::start_rollup;

const MAX_FEE_PER_GAS: u64 = 100000001;

struct TestClient {
    chain_id: u64,
    from_addr: Address,
    contract: SimpleStorageContract,
    client: SignerMiddleware<Provider<Http>, Wallet<SigningKey>>,
}

impl TestClient {
    #[allow(dead_code)]
    async fn new_demo_rollup_client(
        chain_id: u64,
        key: Wallet<SigningKey>,
        from_addr: Address,
        contract: SimpleStorageContract,
        rpc_addr: std::net::SocketAddr,
    ) -> Self {
        let provider =
            Provider::try_from(&format!("http://localhost:{}", rpc_addr.port())).unwrap();

        let client = SignerMiddleware::new_with_provider_chain(provider, key)
            .await
            .unwrap();

        Self {
            chain_id,
            from_addr,
            contract,
            client,
        }
    }

    async fn execute(self) -> Result<(), Box<dyn std::error::Error>> {
        // Deploy contract

        let contract_address = {
            let request = Eip1559TransactionRequest::new()
                .from(self.from_addr)
                .chain_id(self.chain_id)
                .nonce(0u64)
                .max_priority_fee_per_gas(10u64)
                .max_fee_per_gas(MAX_FEE_PER_GAS)
                .gas(900000u64)
                .data(self.contract.byte_code());

            let typed_transaction = TypedTransaction::Eip1559(request);

            let receipt = self
                .client
                .send_transaction(typed_transaction, None)
                .await?
                .await?;

            receipt.unwrap().contract_address.unwrap()
        };

        // Call contract
        let set_arg = 923;
        {
            let request = Eip1559TransactionRequest::new()
                .from(self.from_addr)
                .to(contract_address)
                .chain_id(self.chain_id)
                .nonce(1u64)
                .data(self.contract.set_call_data(set_arg))
                .max_priority_fee_per_gas(10u64)
                .max_fee_per_gas(MAX_FEE_PER_GAS)
                .gas(900000u64);

            let typed_transaction = TypedTransaction::Eip1559(request);

            let _ = self
                .client
                .send_transaction(typed_transaction, None)
                .await
                .unwrap()
                .await;
        }

        // Query contract
        {
            let request = Eip1559TransactionRequest::new()
                .from(self.from_addr)
                .to(contract_address)
                .chain_id(self.chain_id)
                .nonce(2u64)
                .data(self.contract.get_call_data())
                .gas(900000u64);

            let typed_transaction = TypedTransaction::Eip1559(request);

            let response = self.client.call(&typed_transaction, None).await?;

            let resp_array: [u8; 32] = response.to_vec().try_into().unwrap();
            let get_arg = ethereum_types::U256::from(resp_array);

            assert_eq!(set_arg, get_arg.as_u32())
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

    let contract = SimpleStorageContract::new();

    let from_addr = Address::from_str("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap();

    let test_client =
        TestClient::new_demo_rollup_client(chain_id, key, from_addr, contract, rpc_address).await;
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
