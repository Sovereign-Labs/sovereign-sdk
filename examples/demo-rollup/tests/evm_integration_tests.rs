mod test_helpers;
use core::panic;
use std::fs::remove_dir_all;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use celestia::verifier::address::CelestiaAddress;
use const_rollup_config::SEQUENCER_DA_ADDRESS;
use demo_stf::app::App;
use ethers_core::abi::Address;
use ethers_core::k256::ecdsa::SigningKey;
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::Eip1559TransactionRequest;
use ethers_middleware::SignerMiddleware;
use ethers_providers::{Http, Middleware, Provider};
use ethers_signers::{LocalWallet, Signer, Wallet};
use risc0_adapter::host::Risc0Verifier;
use sov_demo_rollup::{get_genesis_config, initialize_ledger, Rollup};
use sov_rollup_interface::mocks::{MockAddress, MockDaService};
use sov_stf_runner::{RollupConfig, RpcConfig, RunnerConfig, StorageConfig};
use test_helpers::SimpleStorageContract;

const MAX_FEE_PER_GAS: u64 = 100000001;

fn create_mock_da_rollup(rollup_config: RollupConfig<()>) -> Rollup<Risc0Verifier, MockDaService> {
    let _ = remove_dir_all(&rollup_config.storage.path);
    let ledger_db = initialize_ledger(rollup_config.storage.path.clone());
    let da_service = MockDaService::default();

    let app = App::new(rollup_config.storage);

    let sequencer_da_address = MockAddress { addr: [99; 32] };
    let genesis_config = get_genesis_config(sequencer_da_address);

    Rollup {
        app,
        da_service,
        ledger_db,
        runner_config: rollup_config.runner,
        genesis_config,
    }
}

async fn start_rollup() {
    let mut mock_path = PathBuf::from("tmp");
    mock_path.push("mocks");

    let rollup_config = RollupConfig {
        storage: StorageConfig { path: mock_path },
        runner: RunnerConfig {
            start_height: 0,
            rpc_config: RpcConfig {
                bind_host: "127.0.0.1".into(),
                bind_port: 12345,
            },
        },
        da: (),
    };

    let rollup = create_mock_da_rollup(rollup_config);
    rollup.run().await.unwrap();
}

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
    ) -> Self {
        let endpoint = format!("http://localhost:{}", 12345);
        let provider = Provider::try_from(endpoint).unwrap();

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

async fn send_tx_test_to_eth() -> Result<(), Box<dyn std::error::Error>> {
    let chain_id: u64 = 1;
    let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
        .parse::<LocalWallet>()
        .unwrap()
        .with_chain_id(chain_id);

    let contract = SimpleStorageContract::new();

    let from_addr = Address::from_str("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap();

    let test_client = TestClient::new_demo_rollup_client(chain_id, key, from_addr, contract).await;
    test_client.execute().await
}

#[tokio::test]
async fn tx_tests() -> Result<(), anyhow::Error> {
    println!("Start");
    let f = tokio::spawn(async {
        start_rollup().await;
    });

    //    tokio::time::sleep(Duration::from_millis(1000)).await;
    //   tokio::time::sleep(Duration::from_millis(3000)).await;
    println!("End");
    send_tx_test_to_eth().await.unwrap();
    //tokio::time::sleep(Duration::from_millis(2000)).await;
    f.abort();

    Ok(())
}
