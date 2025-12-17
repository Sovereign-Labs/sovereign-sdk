use std::net::SocketAddr;

use crate::test_helpers::test_genesis_source;

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, U256};
use alloy_provider::DynProvider;
use alloy_provider::Provider as _;
use alloy_provider::ProviderBuilder;
use alloy_provider::WsConnect;
use reqwest::Url;
use sov_demo_rollup::MockRollupSpec;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_eth_client::SimpleStorageClient;
use sov_evm_test_utils::LegacySimpleStorage;
use sov_full_node_configs::sequencer::TimingOracleConfig;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_modules_api::Spec;
use sov_risc0_adapter::Risc0;
use sov_sequencer::SeqConfigExtension;
use sov_sequencer::SovRateLimiterConfig;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::get_appropriate_rollup_prover_config;
use sov_test_utils::test_rollup::{RollupBuilder, TestRollup};

pub(crate) const SENDER_PRIV_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
pub(crate) const SECONDARY_SENDER_PRIV_KEY: &str =
    "0x96eeea10d406ba7d4e74f7bb9e71b6378165162e4e42fd31c937f7728bbaa7b2";

pub(crate) const EVM_EXTENSION: SeqConfigExtension = SeqConfigExtension {
    max_log_limit: 20000,
    response_size_limit: (1024 * 1024) - (1024 * 30), // Limit our response size to 1MB, leaving 30kb for headers, overhead, and misestimation.
};

/// Starts test rollup node.  
pub(crate) async fn start_node(
    _rollup_prover_config: RollupProverConfig<Risc0>,
    finalization_blocks: u32,
    extension: Option<SeqConfigExtension>,
    timing_oracle_config: Option<TimingOracleConfig>,
    rate_limiter: Option<SovRateLimiterConfig<<MockRollupSpec<Native> as Spec>::Address>>,
) -> TestRollup<MockDemoRollup<Native>> {
    // Don't provide a prover since the EVM is not currently provable
    RollupBuilder::new(
        test_genesis_source(sov_modules_api::OperatingMode::Zk),
        BlockProducingConfig::Periodic {
            block_time_ms: 1_000,
        },
        finalization_blocks,
    )
    .with_preferred_seq_oracle_config(timing_oracle_config)
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .with_rate_limiter(rate_limiter)
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

pub(crate) fn alloy_client_with_signer(socket: SocketAddr, private_key: &str) -> DynProvider {
    let signer: PrivateKeySigner = private_key.parse().unwrap();
    let url = Url::parse(&format!("http://{socket}/rpc")).unwrap();
    ProviderBuilder::new()
        .wallet(signer)
        .connect_http(url)
        .erased()
}

pub(crate) fn alloy_client(socket: SocketAddr) -> DynProvider {
    alloy_client_with_signer(socket, SENDER_PRIV_KEY)
}

pub(crate) fn alloy_client_with_reqwest<B>(
    socket: SocketAddr,
    b: B,
    private_key: &str,
) -> DynProvider
where
    B: FnOnce(reqwest::ClientBuilder) -> reqwest::Client,
{
    let signer: PrivateKeySigner = private_key.parse().unwrap();
    let url = Url::parse(&format!("http://{socket}/rpc")).unwrap();
    ProviderBuilder::new()
        .wallet(signer)
        .with_reqwest(url, b)
        .erased()
}

/// Deploys a test contract on the test rollup.
pub(crate) async fn deploy_contract_check(
    client: &SimpleStorageClient,
) -> Result<Address, Box<dyn std::error::Error>> {
    let runtime_code = client.deploy_contract_call().await?;

    let tx_hash = client.deploy_contract().await?;
    let receipt = client.wait_for_receipt(tx_hash).await;
    let contract_address = receipt.contract_address.unwrap();

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
    let tx_hash = client.set_value(contract_address, set_arg).await;
    client.wait_for_receipt(tx_hash).await;

    let get_arg = client.query_contract(contract_address).await?;
    assert_eq!(U256::from(set_arg), get_arg);

    // Assert storage slot is set
    let storage_slot = 0x0;
    let storage_value = client
        .eth_get_storage_at(contract_address, U256::from(storage_slot))
        .await;
    assert_eq!(storage_value, U256::from(set_arg));

    Ok(())
}

/// Calls `set_values` on the test contract.
pub(crate) async fn set_multiple_values_check(
    client: &SimpleStorageClient,
    contract_address: Address,
    values: Vec<u32>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tx_hashes = client.set_values(contract_address, values).await;

    // Wait for all receipts
    for tx_hash in tx_hashes {
        client.wait_for_receipt(tx_hash).await;
    }

    {
        let get_arg: u32 = client
            .query_contract(contract_address)
            .await?
            .try_into()
            .unwrap();
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
    start_node(config, finalization_blocks, Some(extension), None, None).await
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
