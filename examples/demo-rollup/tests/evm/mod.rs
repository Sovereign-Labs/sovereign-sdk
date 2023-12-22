mod test_client;

use std::net::SocketAddr;
use std::str::FromStr;

use demo_stf::genesis_config::GenesisPaths;
use ethers_core::abi::Address;
use ethers_signers::{LocalWallet, Signer};
use sov_evm::SimpleStorageContract;
use sov_modules_stf_blueprint::kernels::basic::BasicKernelGenesisPaths;
use sov_stf_runner::RollupProverConfig;
use test_client::TestClient;
use tokio::time::{sleep, Duration};

use crate::test_helpers::start_rollup;

#[cfg(feature = "experimental")]
#[tokio::test]
async fn evm_tx_tests() -> Result<(), anyhow::Error> {
    let (port_tx, port_rx) = tokio::sync::oneshot::channel();

    let rollup_task = tokio::spawn(async {
        // Don't provide a prover since the EVM is not currently provable
        start_rollup(
            port_tx,
            GenesisPaths::from_dir("../test-data/genesis/integration-tests"),
            BasicKernelGenesisPaths {
                chain_state: "../test-data/genesis/integration-tests/chain_state.json".into(),
            },
            RollupProverConfig::Skip,
        )
        .await;
    });

    // Wait for rollup task to start:
    let port = port_rx.await.unwrap();
    send_tx_test_to_eth(port).await.unwrap();
    rollup_task.abort();
    Ok(())
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

    let etc_accounts = test_client.eth_accounts().await;
    assert_eq!(vec![from_addr], etc_accounts);

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

    execute(&test_client).await
}

async fn execute(client: &TestClient) -> Result<(), Box<dyn std::error::Error>> {
    // Nonce should be 0 in genesis
    let nonce = client.eth_get_transaction_count(client.from_addr).await;
    assert_eq!(0, nonce);

    // Balance should be > 0 in genesis
    let balance = client.eth_get_balance(client.from_addr).await;
    assert!(balance > ethereum_types::U256::zero());

    let (contract_address, runtime_code) = {
        let runtime_code = client.deploy_contract_call().await?;

        let deploy_contract_req = client.deploy_contract().await?;
        client.send_publish_batch_request().await;

        let contract_address = deploy_contract_req
            .await?
            .unwrap()
            .contract_address
            .unwrap();

        (contract_address, runtime_code)
    };

    // Assert contract deployed correctly
    let code = client.eth_get_code(contract_address).await;
    // code has natural following 0x00 bytes, so we need to trim it
    assert_eq!(code.to_vec()[..runtime_code.len()], runtime_code.to_vec());

    // Nonce should be 1 after the deploy
    let nonce = client.eth_get_transaction_count(client.from_addr).await;
    assert_eq!(1, nonce);

    // Check that the first block has published
    // It should have a single transaction, deploying the contract
    let first_block = client.eth_get_block_by_number(Some("1".to_owned())).await;
    assert_eq!(first_block.number.unwrap().as_u64(), 1);
    assert_eq!(first_block.transactions.len(), 1);

    let set_arg = 923;
    let tx_hash = {
        let set_value_req = client
            .set_value(contract_address, set_arg, None, None)
            .await;
        client.send_publish_batch_request().await;
        set_value_req.await.unwrap().unwrap().transaction_hash
    };

    let get_arg = client.query_contract(contract_address).await?;
    assert_eq!(set_arg, get_arg.as_u32());

    // Assert storage slot is set
    let storage_slot = 0x0;
    let storage_value = client
        .eth_get_storage_at(contract_address, storage_slot.into())
        .await;
    assert_eq!(storage_value, ethereum_types::U256::from(set_arg));

    // Check that the second block has published
    // None should return the latest block
    // It should have a single transaction, setting the value
    let latest_block = client.eth_get_block_by_number_with_detail(None).await;
    assert_eq!(latest_block.number.unwrap().as_u64(), 2);
    assert_eq!(latest_block.transactions.len(), 1);
    assert_eq!(latest_block.transactions[0].hash, tx_hash);

    // This should just pass without error
    client
        .set_value_call(contract_address, set_arg)
        .await
        .unwrap();

    // This call should fail because function does not exist
    let failing_call = client.failing_call(contract_address).await;
    assert!(failing_call.is_err());

    // Create a blob with multiple transactions.
    let mut requests = Vec::default();
    for value in 150..153 {
        let set_value_req = client.set_value(contract_address, value, None, None).await;
        requests.push(set_value_req);
    }

    client.send_publish_batch_request().await;
    client.send_publish_batch_request().await;

    for req in requests {
        req.await.unwrap();
    }

    {
        let get_arg = client.query_contract(contract_address).await?.as_u32();
        // should be one of three values sent in a single block. 150, 151, or 152
        assert!((150..=152).contains(&get_arg));
    }

    {
        let value = 103;

        let tx_hash = {
            let set_value_req = client.set_value_unsigned(contract_address, value).await;
            client.send_publish_batch_request().await;
            set_value_req.await.unwrap().unwrap().transaction_hash
        };

        let latest_block = client.eth_get_block_by_number(None).await;
        assert_eq!(latest_block.transactions.len(), 1);
        assert_eq!(latest_block.transactions[0], tx_hash);

        let get_arg = client.query_contract(contract_address).await?;
        assert_eq!(value, get_arg.as_u32());
    }

    {
        // get initial gas price
        let initial_gas_price = client.eth_gas_price().await;

        // send 100 set transaction with high gas fee in a four batch to increase gas price
        for _ in 0..4 {
            let mut requests = Vec::default();
            for value in 0..25 {
                let set_value_req = client
                    .set_value(contract_address, value, Some(20u64), Some(21u64))
                    .await;
                requests.push(set_value_req);
            }
            client.send_publish_batch_request().await;
            sleep(Duration::from_millis(1000)).await;
        }
        sleep(Duration::from_millis(6000)).await;
        // get gas price
        let latest_gas_price = client.eth_gas_price().await;

        // assert gas price is higher
        // TODO: emulate gas price oracle here to have exact value
        assert!(latest_gas_price > initial_gas_price);
    }

    let first_block = client.eth_get_block_by_number(Some("0".to_owned())).await;
    let second_block = client.eth_get_block_by_number(Some("1".to_owned())).await;

    // assert parent hash works correctly
    assert_eq!(
        first_block.hash.unwrap(),
        second_block.parent_hash,
        "Parent hash should be the hash of the previous block"
    );

    Ok(())
}
