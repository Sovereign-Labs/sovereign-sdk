use std::net::SocketAddr;

use crate::test_helpers::test_genesis_source;

use alloy::signers::local::PrivateKeySigner;
use alloy_provider::DynProvider;
use alloy_provider::Provider as _;
use alloy_provider::ProviderBuilder;
use alloy_provider::WsConnect;
use ethers::core::abi::Address;
use futures::future::join_all;
use reqwest::Url;
use sov_demo_rollup::MockRollupSpec;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_eth_client::SimpleStorageClient;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_risc0_adapter::Risc0;
use sov_sequencer::SeqConfigExtension;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::get_appropriate_rollup_prover_config;
use sov_test_utils::test_rollup::{RollupBuilder, TestRollup};
use sov_test_utils::LegacySimpleStorage;

pub(crate) const SENDER_PRIV_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

pub(crate) const EVM_EXTENSION: SeqConfigExtension = SeqConfigExtension {
    max_log_limit: 20000,
    response_size_limit: (1024 * 1024) - (1024 * 30), // Limit our response size to 1MB, leaving 30kb for headers, overhead, and misestimation.
};

/// Starts test rollup node.  
pub(crate) async fn start_node(
    _rollup_prover_config: RollupProverConfig<Risc0>,
    finalization_blocks: u32,
    extension: Option<SeqConfigExtension>,
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
        c.extension = extension;
    })
    .start()
    .await
    .unwrap()
}

/// Creates a test simple storage client to communicate with the rollup node & SimpleStorage contract.
pub(crate) async fn create_simple_storage_client(
    rest_port: SocketAddr,
    private_key: &str,
) -> SimpleStorageClient {
    let contract = LegacySimpleStorage::default();
    SimpleStorageClient::new(private_key, contract, rest_port).await
}

pub(crate) async fn alloy_ws_client(socket: SocketAddr) -> DynProvider {
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse().unwrap();
    let url = Url::parse(&format!("ws://{socket}/rpc")).unwrap();
    let ws = WsConnect::new(url);
    ProviderBuilder::new()
        .wallet(signer)
        .connect_ws(ws)
        .await
        .unwrap()
        .erased()
}

pub(crate) fn alloy_client(socket: SocketAddr) -> DynProvider {
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse().unwrap();
    let url = Url::parse(&format!("http://{socket}/rpc")).unwrap();
    ProviderBuilder::new()
        .wallet(signer)
        .connect_http(url)
        .erased()
}

/// Deploys a test contract on the test rollup.
pub(crate) async fn deploy_contract_check(
    client: &SimpleStorageClient,
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
    assert_eq!(code[..runtime_code.len()], runtime_code.to_vec());

    Ok(contract_address)
}

/// Calls `set_value` on the test contract.
pub(crate) async fn set_value_check(
    client: &SimpleStorageClient,
    contract_address: Address,
    set_arg: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let _tx_hash = {
        let set_value_req = client.set_value(contract_address, set_arg).await;
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

/// Calls `set_values` on the test contract.
pub(crate) async fn set_multiple_values_check(
    client: &SimpleStorageClient,
    contract_address: Address,
    values: Vec<u32>,
) -> Result<(), Box<dyn std::error::Error>> {
    let requests = client.set_values(contract_address, values).await;

    let receipts: Vec<Result<Option<_>, _>> = join_all(requests).await;
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

pub async fn setup_test_rollup(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> TestRollup<MockDemoRollup<Native>> {
    let host_args = mock_da_risc0_host_args();
    let config = get_appropriate_rollup_prover_config::<MockRollupSpec<Native>>(host_args);
    start_node(config, finalization_blocks, Some(extension)).await
}

pub async fn setup_with_simple_storage(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> (TestRollup<MockDemoRollup<Native>>, SimpleStorageClient, u64) {
    let test_rollup = setup_test_rollup(finalization_blocks, extension).await;
    test_rollup.wait_for_next_blocks(10).await;
    let simple_storage = create_simple_storage_client(test_rollup.http_addr, SENDER_PRIV_KEY).await;
    (test_rollup, simple_storage, config_value!("CHAIN_ID"))
}
