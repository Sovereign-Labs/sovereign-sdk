use std::net::SocketAddr;

use ethers_core::abi::Address;
use ethers_providers::ProviderError;
use ethers_signers::{LocalWallet, Signer};
use futures::future::join_all;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_eth_client::TestClient;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_risc0_adapter::Risc0;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::{RollupBuilder, TestRollup};
use sov_test_utils::SimpleStorageContract;

use crate::test_helpers::test_genesis_source;

/// Starts test rollup node.  
pub(crate) async fn start_node(
    _rollup_prover_config: RollupProverConfig<Risc0>,
    finalization_blocks: u32,
) -> TestRollup<MockDemoRollup<Native>> {
    // Don't provide a prover since the EVM is not currently provable
    RollupBuilder::new(
        test_genesis_source(sov_modules_api::OperatingMode::Zk),
        BlockProducingConfig::Periodic {
            block_time_ms: 1_000,
        },
        finalization_blocks,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 65536;
        c.rollup_prover_config = None; // FIXME(@neysofu): reenable once sov-ethereum is compatible with proof blobs
        c.aggregated_proof_block_jump = 5;
        c.max_infos_in_db = 30;
        c.max_channel_size = 20;
    })
    .start()
    .await
    .unwrap()
}

/// Creates a test client to communicate with the rollup node.
pub(crate) async fn create_test_client(
    rest_port: SocketAddr,
    chain_id: u64,
    private_key: &str,
) -> (TestClient, Address) {
    let key = private_key
        .parse::<LocalWallet>()
        .unwrap()
        .with_chain_id(chain_id);

    let contract = SimpleStorageContract::default();
    let from_addr = key.address();

    let test_client = TestClient::new(chain_id, key, from_addr, contract, rest_port).await;

    let eth_chain_id = test_client.eth_chain_id().await;
    assert_eq!(chain_id, eth_chain_id);

    (test_client, from_addr)
}

/// Deploys a test contract on the test rollup.
pub(crate) async fn deploy_contract_check(
    client: &TestClient,
) -> Result<Address, Box<dyn std::error::Error>> {
    let runtime_code = client.deploy_contract_call().await?;

    let deploy_contract_req = client.deploy_contract().await?;

    let contract_address = deploy_contract_req
        .await?
        .unwrap()
        .contract_address
        .unwrap();

    // Assert contract deployed correctly
    let code = client.eth_get_code(contract_address).await;
    // code has natural following 0x00 bytes, so we need to trim it
    assert_eq!(code.to_vec()[..runtime_code.len()], runtime_code.to_vec());

    Ok(contract_address)
}

/// Calls `set_value` on the test contract.
pub(crate) async fn set_value_check(
    client: &TestClient,
    contract_address: Address,
    set_arg: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let _tx_hash = {
        let set_value_req = client
            .set_value(contract_address, set_arg, None, None)
            .await;
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

    Ok(())
}

/// Calls `set_value` on the test contract with unsigned transaction.
pub(crate) async fn set_value_unsigned_check(
    client: &TestClient,
    contract_address: Address,
    set_arg: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let set_value_req = client.set_value_unsigned(contract_address, set_arg).await;
    set_value_req.await.unwrap().unwrap();

    let get_arg = client.query_contract(contract_address).await?;
    assert_eq!(set_arg, get_arg.as_u32());

    Ok(())
}

/// Calls `set_values` on the test contract.
pub(crate) async fn set_multiple_values_check(
    client: &TestClient,
    contract_address: Address,
    values: Vec<u32>,
) -> Result<(), Box<dyn std::error::Error>> {
    let requests = client
        .set_values(contract_address, values, None, None)
        .await;

    let receipts: Vec<Result<Option<_>, ProviderError>> = join_all(requests).await;
    assert!(receipts
        .into_iter()
        .all(|x| x.is_ok() && x.unwrap().is_some()));

    {
        let get_arg = client.query_contract(contract_address).await?.as_u32();
        // should be one of three values sent in a single block. 150, 151, or 152
        assert!((150..=152).contains(&get_arg));
    }

    Ok(())
}

// TODO: reenable this check by figuring out a way to get finer grained control over preferred batch production.
// /// Checks evm gas evolution.
// pub(crate) async fn gas_check(
//     client: &TestClient,
//     da_service: &StorableMockDaService,
//     contract_address: Address,
// ) -> Result<(), Box<dyn std::error::Error>> {
//     // get initial gas price
//     let initial_base_fee_per_gas = client.eth_gas_price().await;
//
//     // send 10 "set" transactions with high gas fee in 5 batches to increase gas price
//     for _ in 0..5 {
//         let values: Vec<u32> = (0..10).collect();
//         let requests = client
//             .set_values(contract_address, values, Some(200u64), Some(210u128))
//             .await;
//
//         let receipts: Vec<Result<Option<_>, ProviderError>> = join_all(requests).await;
//         assert!(receipts
//             .into_iter()
//             .all(|x| x.is_ok() && x.unwrap().is_some()));
//     }
//     // get gas price
//     let latest_gas_price = client.eth_gas_price().await;
//
//     // assert gas price is higher
//     // TODO: emulate gas price oracle here to have exact value
//     assert!(
//         latest_gas_price > initial_base_fee_per_gas,
//         "Failed gas check initial={:?} latest={:?}",
//         initial_base_fee_per_gas,
//         latest_gas_price,
//     );
//     Ok(())
// }
