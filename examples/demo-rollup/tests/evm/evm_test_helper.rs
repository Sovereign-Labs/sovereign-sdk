use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use crate::test_helpers::test_genesis_source;

use alloy::consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy::eips::Encodable2718;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy_primitives::{hex, Address, Bytes, TxKind, B256, U256, U64};
use alloy_provider::DynProvider;
use alloy_provider::Provider as _;
use alloy_provider::ProviderBuilder;
use alloy_provider::WsConnect;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use reqwest::Client;
use reqwest::Url;
use serde::Serialize;
use serde_json::{json, Value};
use sov_demo_rollup::MockRollupSpec;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_eth_client::SimpleStorageClient;
use sov_evm_test_utils::LegacySimpleStorage;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_modules_api::Spec;
use sov_risc0_adapter::Risc0;
use sov_sequencer::SeqConfigExtension;
use sov_sequencer::SovRateLimiterConfig;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::{
    get_appropriate_rollup_prover_config, RollupBuilder, TestRollup,
};

pub(crate) const SENDER_PRIV_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
pub(crate) const SECONDARY_SENDER_PRIV_KEY: &str =
    "0x96eeea10d406ba7d4e74f7bb9e71b6378165162e4e42fd31c937f7728bbaa7b2";

pub(crate) const EVM_EXTENSION: SeqConfigExtension = SeqConfigExtension {
    max_log_limit: 20000,
    response_size_limit: (1024 * 1024) - (1024 * 30), // Limit our response size to 1MB, leaving 30kb for headers, overhead, and misestimation.
};
pub(crate) const MAX_FEE_PER_GAS: u128 = 1_000_000_000;
pub(crate) const HIGH_MAX_FEE_PER_GAS: u128 = 1_000_000_000_000;
pub(crate) const PAYER_SOV_BANK_BALANCE: u128 = 5_000_000_000_000_000;
pub(crate) const HIGH_PRIORITY_FEE_PER_GAS: u128 = 1;
pub(crate) const MAX_POLL_ATTEMPTS: usize = 100;
pub(crate) const POLL_INTERVAL_MS: u64 = 25;
pub(crate) const INVALID_PARAMS_CODE: i64 = -32602;
pub(crate) const INSUFFICIENT_FUNDS_ERROR: &str = "insufficient funds for gas * price + value";
pub(crate) const FEE_CAP_TOO_LOW_ERROR: &str = "max fee per gas less than block base fee";

/// Starts test rollup node.  
pub(crate) async fn start_node(
    _rollup_prover_config: RollupProverConfig<Risc0>,
    finalization_blocks: u32,
    extension: Option<SeqConfigExtension>,
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

pub(crate) async fn rpc_call(
    client: &Client,
    http_addr: SocketAddr,
    method: &str,
    params: Value,
) -> anyhow::Result<Value> {
    Ok(client
        .post(format!("http://{http_addr}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        }))
        .send()
        .await?
        .json::<Value>()
        .await?)
}

pub(crate) fn hex_u64(value: u64) -> String {
    format!("0x{value:x}")
}

pub(crate) fn hex_word_u64(value: u64) -> String {
    format!("0x{value:064x}")
}

pub(crate) fn hex_u128(value: u128) -> String {
    format!("0x{value:x}")
}

pub(crate) fn parse_hex_u64(value: &str) -> u64 {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.is_empty() {
        return 0;
    }
    u64::from_str_radix(hex, 16).expect("valid u64 hex quantity")
}

pub(crate) fn parse_hex_u128(value: &str) -> u128 {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.is_empty() {
        return 0;
    }
    u128::from_str_radix(hex, 16).expect("valid u128 hex quantity")
}

pub(crate) fn rpc_result_hex(response: &Value) -> String {
    response
        .get("result")
        .and_then(Value::as_str)
        .expect("result should be a hex string")
        .to_string()
}

pub(crate) fn rpc_error_code_from_response(response: &Value, method: &str) -> i64 {
    rpc_error_code(rpc_error_object(response, method))
}

pub(crate) async fn tx_count(
    client: &SimpleStorageClient,
    address: Address,
    block: impl Serialize,
) -> anyhow::Result<u64> {
    let count: U64 = client
        .ws
        .request("eth_getTransactionCount", rpc_params![address, block])
        .await?;
    Ok(count.to::<u64>())
}

pub(crate) async fn raw_signed_eip1559(
    signer: &PrivateKeySigner,
    chain_id: u64,
    nonce: u64,
    gas_limit: u64,
    to: TxKind,
    value: U256,
    input: Bytes,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
) -> anyhow::Result<String> {
    let tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        to,
        value,
        input,
        access_list: Default::default(),
    };
    let sig = signer.sign_hash(&tx.signature_hash()).await?;
    let envelope = TxEnvelope::Eip1559(tx.into_signed(sig));
    Ok(format!("0x{}", hex::encode(envelope.encoded_2718())))
}

pub(crate) fn eth_call_params(from: &str, to: &str, input: &str, block_tag: &str) -> Value {
    json!([{
        "from": from,
        "to": to,
        "gas": "0x7a120",
        "input": input
    }, block_tag])
}

pub(crate) fn rpc_result_str<'a>(response: &'a Value, method: &str) -> &'a str {
    response
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{method} should return result"))
}

pub(crate) fn rpc_error_object<'a>(response: &'a Value, method: &str) -> &'a Value {
    response
        .get("error")
        .unwrap_or_else(|| panic!("{method} should return an error object"))
}

pub(crate) fn rpc_error_code(error: &Value) -> i64 {
    error
        .get("code")
        .and_then(Value::as_i64)
        .expect("error.code should be present")
}

pub(crate) fn rpc_error_message(error: &Value) -> &str {
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

pub(crate) fn rpc_error_data_str(error: &Value) -> Option<&str> {
    error.get("data").and_then(Value::as_str)
}

pub(crate) fn hash_selector(hash: B256) -> Value {
    json!({
        "blockHash": format!("{:#x}", hash),
        "requireCanonical": true
    })
}

pub(crate) async fn poll_until<T, F, Fut, P>(
    mut fetch: F,
    mut predicate: P,
    failure_msg: &str,
) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
    P: FnMut(&T) -> bool,
{
    let mut value = fetch().await?;
    for _ in 0..MAX_POLL_ATTEMPTS {
        if predicate(&value) {
            return Ok(value);
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        value = fetch().await?;
    }

    anyhow::bail!("{failure_msg}")
}

pub(crate) fn assert_invalid_params(response: &Value) {
    let error = rpc_error_object(response, "assert_invalid_params");
    assert_eq!(
        rpc_error_code(error),
        INVALID_PARAMS_CODE,
        "expected JSON-RPC invalid params code"
    );
}

pub(crate) async fn finalized_block_number_and_hash(client: &SimpleStorageClient) -> (u64, B256) {
    let finalized_block = client
        .eth_get_block_by_number(Some("finalized".to_string()))
        .await;
    (finalized_block.header.number, finalized_block.header.hash)
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
    start_node(config, finalization_blocks, Some(extension), None).await
}

pub async fn setup_test_rollup_with_paymaster(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> TestRollup<MockDemoRollup<Native>> {
    let mut paths = crate::test_helpers::test_genesis_paths(sov_modules_api::OperatingMode::Zk);
    paths.paymaster_genesis_path = std::path::PathBuf::from(
        "../test-data/genesis/integration-tests/paymaster_with_payer.json",
    );

    RollupBuilder::new(
        sov_test_utils::test_rollup::GenesisSource::Paths(paths),
        BlockProducingConfig::Periodic {
            block_time_ms: 1_000,
        },
        finalization_blocks,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 65536;
        c.rollup_prover_config = None;
        c.aggregated_proof_block_jump = 5;
        c.max_infos_in_db = 30;
        c.max_channel_size = 20;
        c.extension = Some(extension);
    })
    .start()
    .await
    .unwrap()
}

pub async fn setup_with_simple_storage(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> (TestRollup<MockDemoRollup<Native>>, SimpleStorageClient, u64) {
    let test_rollup = setup_test_rollup(finalization_blocks, extension).await;
    test_rollup.wait_for_rollup_height_advance_by(10).await;
    let simple_storage = create_simple_storage_client(test_rollup.http_addr, SENDER_PRIV_KEY).await;
    (test_rollup, simple_storage, config_value!("CHAIN_ID"))
}
