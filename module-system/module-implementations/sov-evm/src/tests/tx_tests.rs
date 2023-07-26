use std::str::FromStr;

use anvil::NodeConfig;
use ethers_core::abi::Address;
use ethers_core::k256::ecdsa::SigningKey;
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::{Bytes, Eip1559TransactionRequest};
use ethers_core::utils::rlp::Rlp;
use ethers_middleware::SignerMiddleware;
use ethers_providers::{Http, Middleware, Provider};
use ethers_signers::{LocalWallet, Signer, Wallet};

use crate::evm::test_helpers::SimpleStorageContract;

const MAX_FEE_PER_GAS: u64 = 1000000001;

#[tokio::test]
async fn tx_rlp_encoding_test() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = "dcf2cbdd171a21c480aa7f53d77f31bb102282b3ff099c78e3118b37348c72f7"
        .parse::<LocalWallet>()?;
    let from_addr = wallet.address();
    let to_addr = Address::from_str("0x0aa7420c43b8c1a7b165d216948870c8ecfe1ee1")?;
    let data: Bytes = Bytes::from_str(
        "0x6ecd23060000000000000000000000000000000000000000000000000000000000000002",
    )?;

    let tx_request = Eip1559TransactionRequest::new()
        .from(from_addr)
        .chain_id(1u64)
        .nonce(0u64)
        .max_priority_fee_per_gas(413047990155u64)
        .max_fee_per_gas(768658734568u64)
        .gas(184156u64)
        .to(to_addr)
        .value(2000000000000u64)
        .data(data);

    let tx = TypedTransaction::Eip1559(tx_request);

    let sig = wallet.sign_transaction(&tx).await?;
    sig.verify(tx.sighash(), wallet.address())?;

    let rlp_bytes = tx.rlp_signed(&sig);
    let rlp_encoded = Rlp::new(&rlp_bytes);

    let (decoded_tx, decoded_sig) = TypedTransaction::decode_signed(&rlp_encoded)?;
    decoded_sig.verify(decoded_tx.sighash(), wallet.address())?;

    assert_eq!(tx, decoded_tx);
    Ok(())
}

struct TestClient {
    chain_id: u64,
    from_addr: Address,
    contract: SimpleStorageContract,
    client: SignerMiddleware<Provider<Http>, Wallet<SigningKey>>,
}

impl TestClient {
    async fn new_anvil_client(
        chain_id: u64,
        key: Wallet<SigningKey>,
        from_addr: Address,
        contract: SimpleStorageContract,
    ) -> Self {
        let config = NodeConfig {
            chain_id: Some(chain_id),
            ..Default::default()
        };

        let (_api, handle) = anvil::spawn(config).await;

        let provider = Provider::try_from(handle.http_endpoint()).unwrap();

        // let endpoint = format!("http://localhost:{}", 8545);
        // let provider = Provider::try_from(endpoint).unwrap();

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

#[tokio::test]
async fn send_tx_test_to_eth() -> Result<(), Box<dyn std::error::Error>> {
    let chain_id: u64 = 1;
    let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
        .parse::<LocalWallet>()
        .unwrap()
        .with_chain_id(chain_id);

    let contract = SimpleStorageContract::new();

    let from_addr = Address::from_str("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap();

    let test_client = TestClient::new_anvil_client(chain_id, key, from_addr, contract).await;
    // let test_client = TestClient::new_demo_rollup_client(chain_id, key, from_addr, contract).await;
    test_client.execute().await
}
